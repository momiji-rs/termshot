"""Just enough of the GitHub REST API for the action's tests, in memory."""

import base64
import hashlib
import json
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import unquote, urlparse


class State:
    def __init__(self):
        self.blobs, self.trees, self.commits, self.refs = {}, {}, {}, {}
        self.comments, self.pulls, self.branches = [], {}, {"main"}
        self.artifacts = {}  # run id -> [(name, zip bytes)]
        self.calls = []

    def files(self, branch="termshot-assets"):
        if branch not in self.refs:
            return {}
        return {p: self.blobs[s] for p, s in self.trees[self.commits[self.refs[branch]]["tree"]].items()}


def sha(obj):
    return hashlib.sha1(json.dumps(obj, sort_keys=True).encode()).hexdigest()


def serve(state):
    class H(BaseHTTPRequestHandler):
        def log_message(self, *a):
            pass

        def send(self, code, body=None, headers=()):
            raw = body if isinstance(body, bytes) else json.dumps(body).encode() if body is not None else b""
            self.send_response(code)
            for k, v in headers:
                self.send_header(k, v)
            self.send_header("Content-Length", str(len(raw)))
            self.end_headers()
            self.wfile.write(raw)

        def route(self, method):
            u = urlparse(self.path)
            state.calls.append((method, u.path))
            if u.path.startswith("/blobstore/"):
                if self.headers.get("Authorization"):
                    return self.send(400, {"message": "storage refuses Authorization"})
                run, i = u.path.split("/")[2:4]
                return self.send(200, state.artifacts[int(run)][int(i)][1])
            if not self.headers.get("Authorization"):
                return self.send(401, {})
            n = int(self.headers.get("Content-Length") or 0)
            b = json.loads(self.rfile.read(n)) if n else None
            p = [unquote(x) for x in u.path.split("/")[4:]]  # after /repos/o/r
            key = "/".join(p)
            if key == "git/blobs":
                s = hashlib.sha1(b["content"].encode()).hexdigest()
                state.blobs[s] = base64.b64decode(b["content"])
                return self.send(201, {"sha": s})
            if p[:2] == ["git", "blobs"]:
                return self.send(200, {"content": base64.b64encode(state.blobs[p[2]]).decode()})
            if key == "git/trees":
                t = dict(state.trees[b["base_tree"]]) if b.get("base_tree") else {}
                for e in b["tree"]:
                    t[e["path"]] = e["sha"]
                s = sha(t)
                state.trees[s] = t
                return self.send(201, {"sha": s})
            if p[:2] == ["git", "trees"]:
                t = state.trees[p[2]]
                return self.send(200, {"truncated": False, "tree": [
                    {"path": k, "sha": v, "type": "blob"} for k, v in t.items()]})
            if key == "git/commits":
                s = sha(b)
                state.commits[s] = b
                return self.send(201, {"sha": s})
            if p[:2] == ["git", "commits"]:
                return self.send(200, {"tree": {"sha": state.commits[p[2]]["tree"]}})
            if p[:3] == ["git", "ref", "heads"]:
                name = "/".join(p[3:])
                if name not in state.refs:
                    return self.send(404, {})
                return self.send(200, {"object": {"sha": state.refs[name]}})
            if key == "git/refs":
                state.refs[b["ref"][len("refs/heads/"):]] = b["sha"]
                return self.send(201, {})
            if p[:3] == ["git", "refs", "heads"]:
                name = "/".join(p[3:])
                parents = state.commits[b["sha"]]["parents"]
                if not b.get("force") and parents != [state.refs[name]]:
                    return self.send(422, {"message": "not a fast forward"})
                state.refs[name] = b["sha"]
                return self.send(200, {})
            if p[0] == "branches":
                return self.send(200 if "/".join(p[1:]) in state.branches else 404, {})
            if p[0] == "pulls":
                pr = state.pulls.get(int(p[1]))
                return self.send(200, pr) if pr else self.send(404, {})
            if p[:1] == ["issues"] and p[2:] == ["comments"]:
                if method == "GET":
                    return self.send(200, [c for c in state.comments if c["pr"] == int(p[1])])
                c = {"id": len(state.comments) + 1, "pr": int(p[1]), "body": b["body"],
                     "html_url": f"https://x/c{len(state.comments) + 1}"}
                state.comments.append(c)
                return self.send(201, c)
            if p[:2] == ["issues", "comments"]:
                c = state.comments[int(p[2]) - 1]
                c["body"] = b["body"]
                return self.send(200, c)
            if p[:2] == ["actions", "runs"]:
                arts = state.artifacts.get(int(p[2]), [])
                return self.send(200, {"total_count": len(arts), "artifacts": [
                    {"id": int(p[2]) * 100 + i, "name": n, "size_in_bytes": len(z), "expired": False}
                    for i, (n, z) in enumerate(arts)]})
            if p[:2] == ["actions", "artifacts"]:
                aid = int(p[2])
                host = self.headers["Host"]
                return self.send(302, b"", [("Location", f"http://{host}/blobstore/{aid // 100}/{aid % 100}")])
            self.send(404, {"path": self.path})

        def do_GET(self):
            self.route("GET")

        def do_POST(self):
            self.route("POST")

        def do_PATCH(self):
            self.route("PATCH")

    srv = ThreadingHTTPServer(("127.0.0.1", 0), H)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv
