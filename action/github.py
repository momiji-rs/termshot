"""The GitHub REST calls the action makes, and the assets branch it keeps.

The assets branch is content-addressed: every PNG, text and JSON is stored once as
objects/<sha256[:2]>/<sha256>.<ext>, and manifests name them:

  baseline/<id>/<branch>.json   the screens of the last push to <branch>
  pr/<id>/<number>.json         the screens last shown on pull request <number>

A file never changes once written, so image URLs can name the branch instead of a
commit, and pruning can squash the history without breaking a comment it keeps.
"""

import base64
import hashlib
import io
import json
import os
import time
import urllib.error
import urllib.request
import zipfile

API = os.environ.get("GITHUB_API_URL", "https://api.github.com")


class HTTPError(RuntimeError):
    def __init__(self, method, path, code, detail):
        super().__init__(f"{method} {path}: HTTP {code}: {detail}")
        self.code = code


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


class GitHub:
    def __init__(self, token, repo):
        self.token, self.repo = token, repo

    def request(self, method, path, body=None, accept="application/vnd.github+json",
                redirect=True):
        url = path if path.startswith("http") else f"{API}/repos/{self.repo}{path}"
        data = json.dumps(body).encode() if body is not None else None
        req = urllib.request.Request(url, data=data, method=method, headers={
            "Authorization": f"Bearer {self.token}",
            "Accept": accept,
            "X-GitHub-Api-Version": "2022-11-28",
            "User-Agent": "termshot-action",
        })
        opener = urllib.request.build_opener() if redirect else urllib.request.build_opener(_NoRedirect)
        for attempt in range(4):
            try:
                with opener.open(req, timeout=60) as r:
                    return r.status, dict(r.headers), r.read()
            except urllib.error.HTTPError as e:
                if not redirect and e.code in (301, 302, 303, 307, 308):
                    return e.code, dict(e.headers), b""
                if e.code in (502, 503, 504) and attempt < 3:
                    time.sleep(1 + attempt * 2)
                    continue
                detail = e.read().decode(errors="replace")[:300]
                raise HTTPError(method, path, e.code, detail) from None

    def call(self, method, path, body=None, ok404=False):
        try:
            _, _, raw = self.request(method, path, body)
        except HTTPError as e:
            if ok404 and e.code == 404:
                return None
            raise
        return json.loads(raw) if raw else None

    def paginate(self, path):
        page = 1
        while True:
            sep = "&" if "?" in path else "?"
            batch = self.call("GET", f"{path}{sep}per_page=100&page={page}")
            items = batch if isinstance(batch, list) else next(
                v for k, v in batch.items() if isinstance(v, list))
            yield from items
            if len(items) < 100:
                return
            page += 1

    def upsert_comment(self, number, marker, body):
        for c in self.paginate(f"/issues/{number}/comments"):
            if marker in (c.get("body") or ""):
                return self.call("PATCH", f"/issues/comments/{c['id']}", {"body": body})
        return self.call("POST", f"/issues/{number}/comments", {"body": body})

    def artifact(self, run_id, name, limit):
        """The files of artifact `name` of workflow run `run_id`, as {name: bytes},
        refusing archives over `limit` bytes. Nothing is written to disk."""
        found = [a for a in self.paginate(f"/actions/runs/{run_id}/artifacts")
                 if a["name"] == name and not a.get("expired")]
        if not found:
            return None
        art = found[0]
        if art["size_in_bytes"] > limit:
            raise RuntimeError(f"artifact {name} is {art['size_in_bytes']} bytes, over {limit}")
        # The API redirects to blob storage, which refuses our Authorization header,
        # so follow the redirect by hand without it.
        code, headers, raw = self.request("GET", f"/actions/artifacts/{art['id']}/zip",
                                          redirect=False)
        if code in (301, 302, 303, 307, 308):
            req = urllib.request.Request(headers["Location"],
                                         headers={"User-Agent": "termshot-action"})
            with urllib.request.urlopen(req, timeout=120) as r:
                raw = r.read(limit + 1)
        if len(raw) > limit:
            raise RuntimeError(f"artifact {name} is over {limit} bytes")
        files = {}
        with zipfile.ZipFile(io.BytesIO(raw)) as z:
            total = 0
            for info in z.infolist():
                if info.is_dir():
                    continue
                total += info.file_size
                if total > limit:
                    raise RuntimeError(f"artifact {name} unpacks to over {limit} bytes")
                files[info.filename] = z.read(info)
        return files


class Store:
    """The assets branch: read its tree once, stage files, commit them in one go."""

    def __init__(self, gh, branch):
        self.gh, self.branch = gh, branch
        self.staged = {}
        self.load()

    def load(self):
        self.head = self.gh.call("GET", f"/git/ref/heads/{self.branch}", ok404=True)
        self.head = self.head["object"]["sha"] if self.head else None
        self.tree, self.truncated = {}, False
        if self.head:
            tree = self.gh.call("GET", f"/git/commits/{self.head}")["tree"]["sha"]
            t = self.gh.call("GET", f"/git/trees/{tree}?recursive=1")
            self.truncated = t.get("truncated", False)
            self.tree = {e["path"]: e["sha"] for e in t["tree"] if e["type"] == "blob"}

    def read(self, path):
        if path in self.staged:
            return self.staged[path]
        sha = self.tree.get(path)
        if not sha:
            return None
        return base64.b64decode(self.gh.call("GET", f"/git/blobs/{sha}")["content"])

    def put(self, path, data):
        self.staged[path] = data

    def put_object(self, data, ext):
        h = hashlib.sha256(data).hexdigest()
        path = f"objects/{h[:2]}/{h}.{ext}"
        if path not in self.tree:
            self.staged[path] = data
        return path

    def _blob(self, data):
        return self.gh.call("POST", "/git/blobs", {
            "content": base64.b64encode(data).decode(), "encoding": "base64"})["sha"]

    def commit(self, message):
        """Commit the staged files on top of the branch, creating it as an orphan if
        needed, and retrying when another job moved it meanwhile."""
        if not self.staged:
            return self.head
        entries = [{"path": p, "mode": "100644", "type": "blob", "sha": self._blob(d)}
                   for p, d in self.staged.items()]
        for attempt in range(6):
            body = {"tree": entries}
            if self.head:
                body["base_tree"] = self.gh.call("GET", f"/git/commits/{self.head}")["tree"]["sha"]
            tree = self.gh.call("POST", "/git/trees", body)["sha"]
            commit = self.gh.call("POST", "/git/commits", {
                "message": message, "tree": tree, "parents": [self.head] if self.head else []})["sha"]
            try:
                if self.head:
                    self.gh.call("PATCH", f"/git/refs/heads/{self.branch}", {"sha": commit})
                else:
                    self.gh.call("POST", "/git/refs", {"ref": f"refs/heads/{self.branch}",
                                                      "sha": commit})
                break
            except HTTPError as e:
                if e.code != 422 or attempt == 5:
                    raise
                time.sleep(1 + attempt)
                self.load()
        for e in entries:
            self.tree[e["path"]] = e["sha"]
        self.staged.clear()
        self.head = commit
        return commit

    def replace(self, keep, message):
        """Make the branch one parentless commit holding only the paths in `keep`, so
        the dropped files stop taking space. Gives up if the branch moved meanwhile."""
        entries = [{"path": p, "mode": "100644", "type": "blob", "sha": self.tree[p]}
                   for p in sorted(keep)]
        tree = self.gh.call("POST", "/git/trees", {"tree": entries})["sha"]
        commit = self.gh.call("POST", "/git/commits", {"message": message, "tree": tree,
                                                       "parents": []})["sha"]
        now = self.gh.call("GET", f"/git/ref/heads/{self.branch}")["object"]["sha"]
        if now != self.head:
            return False
        self.gh.call("PATCH", f"/git/refs/heads/{self.branch}", {"sha": commit, "force": True})
        self.head = commit
        self.tree = {p: self.tree[p] for p in keep}
        return True
