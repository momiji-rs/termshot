#!/usr/bin/env python3
"""termshot GitHub Action: capture terminal screens, render them, show them on the PR.

Standard library only, so it runs on any hosted runner without a setup step.

  capture   run each shot's command in a real PTY of the given size and keep the bytes
  render    termshot each log into a PNG and its --text
  compare   against the baseline stored for the base branch (exact: termshot is
            deterministic, so the same screen gives the same bytes)
  publish   push the PNGs to an orphan assets branch and upsert one sticky comment

Inputs arrive as INPUT_* environment variables (set by action.yml). Run locally with
TERMSHOT_DRY_RUN=1 to stop after rendering and print the comment it would post.
"""

import base64
import difflib
import errno
import fcntl
import glob
import hashlib
import json
import os
import re
import select
import signal
import struct
import subprocess
import sys
import termios
import time
import urllib.error
import urllib.request

API = os.environ.get("GITHUB_API_URL", "https://api.github.com")
SERVER = os.environ.get("GITHUB_SERVER_URL", "https://github.com")
MARK = "<!-- termshot-action:{id} -->"
NAME_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")


def inp(name, default=""):
    return os.environ.get("INPUT_" + name.upper().replace("-", "_"), default).strip()


def log(msg):
    print(msg, file=sys.stderr, flush=True)


def fail(msg):
    print("::error::" + msg, flush=True)
    sys.exit(1)


# ---------------------------------------------------------------- capture


def capture(command, cols, rows, timeout, out_path):
    """Run `command` under bash in a PTY of cols x rows and write what it printed.

    A command still running at `timeout` is a TUI showing its screen: recording stops
    first and the process group is killed after, so its exit cleanup (leaving the
    alternate screen, clearing) never reaches the log. Returns (exit code or None if
    it timed out, seconds).
    """
    master, slave = os.openpty()
    # Size the terminal before the program starts, so its first query sees it.
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
    env = dict(os.environ, TERM="xterm-256color", COLUMNS=str(cols), LINES=str(rows))

    def child_setup():
        os.setsid()
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)

    start = time.monotonic()
    proc = subprocess.Popen(
        ["bash", "-c", command],
        stdin=slave, stdout=slave, stderr=slave,
        env=env, preexec_fn=child_setup, close_fds=True,
    )
    os.close(slave)
    chunks, timed_out = [], False
    while True:
        left = start + timeout - time.monotonic()
        if left <= 0:
            timed_out = True
            break
        ready, _, _ = select.select([master], [], [], min(left, 0.25))
        if not ready:
            # The child may have exited while a grandchild keeps the PTY open.
            if proc.poll() is not None and not select.select([master], [], [], 0.05)[0]:
                break
            continue
        try:
            data = os.read(master, 65536)
        except OSError as e:
            if e.errno == errno.EIO:  # Linux: every slave fd closed
                break
            raise
        if not data:  # macOS
            break
        chunks.append(data)
    if not timed_out:
        try:
            proc.wait(timeout=2)
        except subprocess.TimeoutExpired:
            pass
    if proc.poll() is None:
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except (ProcessLookupError, PermissionError):
            proc.kill()
    code = proc.wait()
    os.close(master)
    with open(out_path, "wb") as f:
        f.write(b"".join(chunks))
    return (None if timed_out else code), time.monotonic() - start


def parse_shots(text):
    shots = []
    for n, line in enumerate(text.splitlines(), 1):
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        name, sep, command = line.partition(":")
        name, command = name.strip(), command.strip()
        if not sep or not command or not NAME_RE.match(name):
            fail(f"shots line {n}: expected 'name: command', got {line!r}")
        shots.append((name, command))
    return shots


# ---------------------------------------------------------------- GitHub API


class GitHub:
    def __init__(self, token, repo):
        self.token, self.repo = token, repo

    def call(self, method, path, body=None, ok404=False):
        url = path if path.startswith("http") else f"{API}/repos/{self.repo}{path}"
        data = json.dumps(body).encode() if body is not None else None
        req = urllib.request.Request(url, data=data, method=method, headers={
            "Authorization": f"Bearer {self.token}",
            "Accept": "application/vnd.github+json",
            "X-GitHub-Api-Version": "2022-11-28",
            "User-Agent": "termshot-action",
        })
        try:
            with urllib.request.urlopen(req, timeout=60) as r:
                raw = r.read()
                return json.loads(raw) if raw else None
        except urllib.error.HTTPError as e:
            if ok404 and e.code == 404:
                return None
            detail = e.read().decode(errors="replace")[:500]
            raise RuntimeError(f"{method} {path}: HTTP {e.code}: {detail}") from None

    def file_at(self, ref, path):
        r = self.call("GET", f"/contents/{path}?ref={ref}", ok404=True)
        return base64.b64decode(r["content"]) if r and "content" in r else None

    def head_of(self, branch):
        r = self.call("GET", f"/git/ref/heads/{branch}", ok404=True)
        return r["object"]["sha"] if r else None

    def commit_files(self, branch, files, message):
        """Add `files` ({path: bytes}) to `branch` in one commit, creating it as an
        orphan if needed. Retries when another job moved the branch meanwhile."""
        blobs = {}
        for path, data in files.items():
            b = self.call("POST", "/git/blobs",
                          {"content": base64.b64encode(data).decode(), "encoding": "base64"})
            blobs[path] = b["sha"]
        tree = [{"path": p, "mode": "100644", "type": "blob", "sha": s} for p, s in blobs.items()]
        for attempt in range(5):
            parent = self.head_of(branch)
            body = {"tree": tree}
            if parent:
                body["base_tree"] = self.call("GET", f"/git/commits/{parent}")["tree"]["sha"]
            t = self.call("POST", "/git/trees", body)
            c = self.call("POST", "/git/commits", {
                "message": message, "tree": t["sha"], "parents": [parent] if parent else []})
            try:
                if parent:
                    self.call("PATCH", f"/git/refs/heads/{branch}", {"sha": c["sha"]})
                else:
                    self.call("POST", "/git/refs", {"ref": f"refs/heads/{branch}", "sha": c["sha"]})
                return c["sha"]
            except RuntimeError as e:
                if "HTTP 422" not in str(e) or attempt == 4:
                    raise
                time.sleep(1 + attempt)

    def upsert_comment(self, number, marker, body):
        page = 1
        while True:
            batch = self.call("GET", f"/issues/{number}/comments?per_page=100&page={page}")
            for c in batch:
                if marker in (c.get("body") or ""):
                    return self.call("PATCH", f"/issues/comments/{c['id']}", {"body": body})
            if len(batch) < 100:
                return self.call("POST", f"/issues/{number}/comments", {"body": body})
            page += 1


# ---------------------------------------------------------------- report


def text_diff(old, new, name):
    lines = difflib.unified_diff(old.splitlines(), new.splitlines(),
                                 f"base/{name}", f"head/{name}", lineterm="", n=1)
    return "\n".join(list(lines)[2:])  # the ---/+++ header says nothing here


def fence(text, lang=""):
    ticks = "```"
    while ticks in text:
        ticks += "`"
    return f"{ticks}{lang}\n{text}\n{ticks}"


def img(url, alt, width=None):
    w = f' width="{width}"' if width else ""
    return f'<img src="{url}" alt="{alt}"{w}>'


def report(ctx, shots, base, url_of):
    """Markdown for the comment and the job summary. `url_of(kind, name)` gives the
    image URL for 'head' or 'base', or None when images aren't published."""
    changed = [s for s in shots if s["status"] == "changed"]
    new = [s for s in shots if s["status"] == "new"]
    same = [s for s in shots if s["status"] == "unchanged"]
    removed = sorted(set(base) - {s["name"] for s in shots}) if base is not None else []

    counts = [f"{len(shots)} screen{'s' * (len(shots) != 1)}"]
    if base is not None:
        counts += [f"{len(changed)} changed"] if changed else []
        counts += [f"{len(new)} new"] if new else []
        counts += [f"{len(removed)} removed"] if removed else []
        if not changed and not new and not removed:
            counts.append("no visual changes")
    out = [MARK.format(id=ctx["id"]), f"### 📸 termshot · {' · '.join(counts)}", ""]
    if base is None and ctx.get("base_ref"):
        out += [f"_No baseline for `{ctx['base_ref']}` yet: it is stored when this "
                f"workflow runs on a push to `{ctx['base_ref']}`._", ""]

    def meta(s):
        bits = [f"`{s['command']}`" if s.get("command") else f"`{s['source']}`"]
        if s.get("exit") is None and s.get("command"):
            bits.append(f"captured at {s['timeout']:g}s")
        elif s.get("exit"):
            bits.append(f"⚠️ exit {s['exit']}")
        return " · ".join(bits)

    def text_block(s):
        return f"<details><summary>text</summary>\n\n{fence(s['text'])}\n\n</details>\n"

    for s in changed:
        out += [f"#### ✏️ `{s['name']}` changed", meta(s), ""]
        hu, bu = url_of("head", s["name"]), url_of("base", s["name"])
        if hu and bu:
            out += ["| before | after |", "|---|---|",
                    f"| {img(bu, s['name'] + ' before')} | {img(hu, s['name'] + ' after')} |", ""]
        d = text_diff(s["base_text"], s["text"], s["name"])
        if d:
            out += ["<details open><summary>text diff</summary>", "", fence(d, "diff"),
                    "", "</details>", ""]
        else:
            out += ["_Same text; colours or attributes changed._", ""]
    for s in new:
        title = "🆕" if base is not None else "🖥️"
        out += [f"#### {title} `{s['name']}`", meta(s), ""]
        u = url_of("head", s["name"])
        out += [img(u, s["name"]), ""] if u else []
        out += [text_block(s)]
    if same:
        out += [f"<details><summary>✅ {len(same)} unchanged: "
                + ", ".join(f"<code>{s['name']}</code>" for s in same) + "</summary>", ""]
        for s in same:
            u = url_of("head", s["name"])
            out += [f"**`{s['name']}`** · {meta(s)}", ""]
            out += [img(u, s["name"], 480), ""] if u else [fence(s["text"]), ""]
        out += ["</details>", ""]
    if removed:
        out += ["🗑️ removed: " + ", ".join(f"`{n}`" for n in removed), ""]
    foot = f"<sub>Rendered by [termshot]({SERVER}/momiji-rs/termshot) {ctx['version']}"
    if ctx.get("sha"):
        foot += f" at {ctx['sha'][:7]}"
    if ctx.get("run_url"):
        foot += f" · [run]({ctx['run_url']})"
    out.append(foot + "</sub>")
    return "\n".join(out)


# ---------------------------------------------------------------- main


def main():
    termshot = inp("termshot", "termshot")
    size = inp("size", "100x30")
    m = re.fullmatch(r"(\d+)x(\d+)", size)
    if not m:
        fail(f"size must be COLSxROWS, got {size!r}")
    cols, rows = int(m[1]), int(m[2])
    timeout = float(inp("timeout", "10") or 10)
    out_dir = os.path.abspath(inp("output-dir", "termshot-out"))
    shot_id = inp("id", "termshot")
    if not NAME_RE.match(shot_id):
        fail(f"id must match {NAME_RE.pattern}")
    os.makedirs(out_dir, exist_ok=True)
    set_output("dir", out_dir)

    render_args = ["--size", size, "--px", inp("px", "28") or "28"]
    if inp("font"):
        render_args += ["--font", inp("font")]
    if inp("fallback-font"):
        render_args += ["--fallback-font", inp("fallback-font")]
    render_args += inp("args").split()

    shots = [{"name": n, "command": c, "timeout": timeout} for n, c in parse_shots(inp("shots"))]
    for pattern in inp("logs").split():
        matches = sorted(glob.glob(pattern, recursive=True))
        if not matches:
            fail(f"logs: {pattern!r} matched nothing")
        for path in matches:
            name = re.sub(r"[^A-Za-z0-9._-]", "-", os.path.splitext(os.path.basename(path))[0])
            shots.append({"name": name, "source": path, "log": path})
    if not shots:
        fail("give at least one of `shots` or `logs`")
    names = [s["name"] for s in shots]
    if len(set(names)) != len(names):
        fail("shot names must be unique: " + ", ".join(sorted({n for n in names if names.count(n) > 1})))

    version = subprocess.run([termshot, "--version"], capture_output=True, text=True).stdout.split()[-1]
    for s in shots:
        if "command" in s:
            s["log"] = os.path.join(out_dir, s["name"] + ".pty")
            s["exit"], secs = capture(s["command"], cols, rows, timeout, s["log"])
            state = "timed out (kept the screen)" if s["exit"] is None else f"exit {s['exit']}"
            log(f"captured {s['name']}: {state}, {secs:.1f}s")
        png = os.path.join(out_dir, s["name"] + ".png")
        txt = os.path.join(out_dir, s["name"] + ".txt")
        r = subprocess.run([termshot, *render_args, "--text", txt, s["log"], png],
                           capture_output=True, text=True)
        if r.stderr.strip():
            log(r.stderr.strip())
        if r.returncode:
            fail(f"termshot failed on {s['name']} (exit {r.returncode})")
        with open(png, "rb") as f:
            s["png"] = f.read()
        with open(txt, encoding="utf-8", errors="replace") as f:
            s["text"] = f.read().rstrip("\n")
        s["sha"] = hashlib.sha256(s["png"]).hexdigest()

    event = os.environ.get("GITHUB_EVENT_NAME", "")
    payload = {}
    if os.environ.get("GITHUB_EVENT_PATH") and os.path.exists(os.environ["GITHUB_EVENT_PATH"]):
        with open(os.environ["GITHUB_EVENT_PATH"]) as f:
            payload = json.load(f)
    pr = payload.get("pull_request")
    repo = os.environ.get("GITHUB_REPOSITORY", "")
    run_url = (f"{SERVER}/{repo}/actions/runs/{os.environ['GITHUB_RUN_ID']}"
               if os.environ.get("GITHUB_RUN_ID") else "")
    ctx = {"id": shot_id, "version": version, "run_url": run_url,
           "sha": pr["head"]["sha"] if pr else os.environ.get("GITHUB_SHA", ""),
           "base_ref": pr["base"]["ref"] if pr else ""}
    branch = inp("assets-branch", "termshot-assets")
    private = (payload.get("repository") or {}).get("private", True)
    token = inp("github-token")
    gh = GitHub(token, repo) if token and repo and not os.environ.get("TERMSHOT_DRY_RUN") else None

    # Compare against the baseline of the base branch (on a PR) or of this branch.
    base_branch = ctx["base_ref"] or os.environ.get("GITHUB_REF_NAME", "")
    base_dir = f"baseline/{shot_id}/{base_branch}"
    base, base_commit = None, None
    if gh and base_branch:
        base_commit = gh.head_of(branch)
        if base_commit:
            raw = gh.file_at(base_commit, f"{base_dir}/manifest.json")
            base = json.loads(raw)["shots"] if raw else None
    for s in shots:
        b = (base or {}).get(s["name"])
        if not b:
            s["status"] = "new"
        elif b["sha"] == s["sha"]:
            s["status"] = "unchanged"
        else:
            s["status"] = "changed"
            s["base_text"] = (gh.file_at(base_commit, f"{base_dir}/{s['name']}.txt") or b"").decode(
                "utf-8", "replace")

    def raw_url(commit, path):
        # Public repos: raw.githubusercontent.com, proxied by camo. Private repos: a
        # github.com URL, which the viewer's own session can open.
        if private:
            return f"{SERVER}/{repo}/raw/{commit}/{path}"
        return f"https://raw.githubusercontent.com/{repo}/{commit}/{path}"

    can_write = bool(gh) and not (pr and pr["head"]["repo"]["full_name"] != repo)
    published = None
    if can_write and inp("publish", "true") != "false":
        files = {}
        if pr:
            prefix = f"pr/{shot_id}/{pr['number']}/{ctx['sha'][:12]}"
        else:
            prefix = base_dir
        for s in shots:
            if pr and s["status"] == "unchanged":
                continue  # the comment shows the baseline's copy
            files[f"{prefix}/{s['name']}.png"] = s["png"]
            files[f"{prefix}/{s['name']}.txt"] = s["text"].encode()
        if not pr:
            manifest = {"version": version, "commit": ctx["sha"],
                        "shots": {s["name"]: {"sha": s["sha"]} for s in shots}}
            files[f"{prefix}/manifest.json"] = json.dumps(manifest, indent=1).encode()
        try:
            if not files:
                published = base_commit
            else:
                published = gh.commit_files(branch, files, f"termshot {shot_id}: {ctx['sha'][:12]}")
                log(f"pushed {len(files)} files to {branch} at {published[:12]}")
        except RuntimeError as e:
            log(f"::warning::could not push screenshots to {branch} ({e}); "
                "the action needs `permissions: contents: write`")

    def url_of(kind, name):
        if kind == "head" and status[name] == "unchanged" and pr:
            kind = "base"
        if kind == "head" and published:
            return raw_url(published, f"{prefix}/{name}.png")
        if kind == "base" and base_commit:
            return raw_url(base_commit, f"{base_dir}/{name}.png")
        return None

    status = {s["name"]: s["status"] for s in shots}
    body = report(ctx, shots, base if (pr or not gh) else None, url_of)

    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as f:
            f.write(body + "\n")
    if os.environ.get("TERMSHOT_DRY_RUN"):
        print(body)
    comment_mode = inp("comment", "auto")
    if pr and gh and comment_mode != "never":
        if can_write:
            try:
                c = gh.upsert_comment(pr["number"], MARK.format(id=shot_id), body)
                log(f"comment: {c['html_url']}")
                set_output("comment-url", c["html_url"])
            except RuntimeError as e:
                log(f"::warning::could not comment ({e}); the action needs "
                    "`permissions: pull-requests: write`")
        else:
            log("::notice::pull request from a fork: the token cannot write, so the "
                "screens are in the job summary and the artifact instead")

    n_changed = sum(s["status"] == "changed" for s in shots)
    set_output("changed", str(n_changed))
    if n_changed and inp("fail-on-change") == "true":
        fail(f"{n_changed} screen(s) changed")


def set_output(key, value):
    if os.environ.get("GITHUB_OUTPUT"):
        with open(os.environ["GITHUB_OUTPUT"], "a") as f:
            f.write(f"{key}={value}\n")


if __name__ == "__main__":
    main()
