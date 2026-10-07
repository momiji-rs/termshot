#!/usr/bin/env python3
"""termshot GitHub Action: screenshot terminal screens and show on the PR what changed.

Standard library only, so it runs on any hosted runner without a setup step.

mode run (pull_request, push, anything but workflow_run):
  capture   run each shot's command in a real PTY of its size, driven by its steps
  render    termshot each log into a PNG, its --text and its --json
  compare   against the baseline stored for the base branch. termshot is
            deterministic, so the same screen gives the same bytes and a change
            is exact. A changed screen gets a cell diff that termshot draws too.
  publish   store the files on the assets branch and upsert one sticky comment.
            A push stores the branch's baseline instead, and prunes.
  When the token can't write (a fork, Dependabot), it only renders, and leaves the
  logs in the artifact for mode publish.

mode publish (workflow_run, after the run above):
  Takes the logs from that run's artifact as untrusted data: checks them, matches
  the pull request through the API, renders them with its own termshot, then
  compares and publishes as above. Nothing from the artifact is executed.

Inputs arrive as INPUT_* environment variables (set by action.yml). Run locally
with TERMSHOT_DRY_RUN=1 to stop after rendering and print the comment.
"""

import datetime
import glob
import hashlib
import json
import os
import re
import subprocess
import sys
from urllib.parse import quote

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import capture  # noqa: E402
import report  # noqa: E402
from github import GitHub, HTTPError, Store  # noqa: E402

SERVER = os.environ.get("GITHUB_SERVER_URL", "https://github.com")
BUNDLE = ".termshot-bundle.json"  # starts with '.', so no screen name can be it
ARTIFACT_LIMIT = 64 << 20
MAX_SCREENS = 200


def inp(name, default=""):
    v = os.environ.get("INPUT_" + name.upper().replace("-", "_"), "").strip()
    return v or default


def log(msg):
    print(msg, file=sys.stderr, flush=True)


def fail(msg):
    print("::error::" + msg, flush=True)
    sys.exit(1)


def set_output(key, value):
    if os.environ.get("GITHUB_OUTPUT"):
        with open(os.environ["GITHUB_OUTPUT"], "a") as f:
            f.write(f"{key}={value}\n")


def now():
    return datetime.datetime.now(datetime.timezone.utc).replace(microsecond=0)


class Renderer:
    def __init__(self, termshot, out_dir):
        self.termshot, self.out_dir = termshot, out_dir
        args = ["--px", inp("px", "28")]
        if inp("font"):
            args += ["--font", inp("font")]
        if inp("fallback-font"):
            args += ["--fallback-font", inp("fallback-font")]
        self.args = args + inp("args").split()
        r = subprocess.run([termshot, "--version"], capture_output=True, text=True)
        self.version = r.stdout.split()[-1] if r.returncode == 0 else "?"

    def render(self, screen, extra=()):
        """Render screen['log'] (bytes) at screen['size'], filling png/text/json."""
        cols, rows = screen["size"]
        stem = os.path.join(self.out_dir, screen["name"])
        with open(stem + ".pty", "wb") as f:
            f.write(screen["log"])
        r = subprocess.run([self.termshot, *self.args, "--size", f"{cols}x{rows}", *extra,
                            "--text", stem + ".txt", "--json", stem + ".json",
                            stem + ".pty", stem + ".png"], capture_output=True, text=True)
        if r.stderr.strip():
            log(f"{screen['name']}: {r.stderr.strip()}")
        if r.returncode:
            raise RuntimeError(f"termshot failed on {screen['name']} (exit {r.returncode})")
        for key, ext in (("png", "png"), ("json", "json")):
            with open(f"{stem}.{ext}", "rb") as f:
                screen[key] = f.read()
        with open(stem + ".txt", encoding="utf-8", errors="replace") as f:
            screen["text"] = f.read().rstrip("\n")

    def diff_png(self, screen, base_json):
        data, changed = report.diff_log(json.loads(screen["json"]), json.loads(base_json))
        d = {"name": screen["name"] + ".diff", "size": screen["size"], "log": data}
        self.render(d, extra=("--cursor", "none"))
        return d["png"], report.describe(changed)


# ---------------------------------------------------------------- gather screens


def gather(renderer, default_size, timeout):
    screens = []
    for shot in capture.parse_shots(inp("shots"), default_size):
        got, code = capture.run(shot, timeout, renderer.termshot)
        for g in got:
            g.update(label=shot["command"], size=shot["size"], steps=len(shot["steps"]),
                     exit=code)
            log(f"captured {g['name']}: " + ("running" if code is None else f"exit {code}")
                + (f", {g['note']}" if g["note"] else ""))
        screens += got
    for item in inp("logs").split():
        pattern, _, size = item.partition("@")
        size = capture.parse_size(size, default_size)
        matches = sorted(glob.glob(pattern, recursive=True))
        if not matches:
            raise capture.SpecError(f"logs: {pattern!r} matched nothing")
        for path in matches:
            name = re.sub(r"[^A-Za-z0-9._-]", "-", os.path.splitext(os.path.basename(path))[0])
            with open(path, "rb") as f:
                screens.append({"name": name, "label": path, "size": size, "log": f.read(),
                                "note": "", "steps": 0, "exit": None})
    return screens


def check_names(screens):
    if not screens:
        raise capture.SpecError("give at least one of `shots` or `logs`")
    names = [s["name"] for s in screens]
    dup = sorted({n for n in names if names.count(n) > 1})
    if dup:
        raise capture.SpecError("screen names must be unique: " + ", ".join(dup))


# ---------------------------------------------------------------- compare and publish


def raw_url(repo, branch, path, private):
    # Public: raw.githubusercontent.com, fetched through GitHub's image proxy.
    # Private: a github.com URL, which works for a signed-in viewer with access.
    if private:
        return f"{SERVER}/{repo}/raw/{branch}/{path}"
    return f"https://raw.githubusercontent.com/{repo}/{branch}/{path}"


def compare_and_publish(ctx, screens, renderer, gh, store, pr):
    """Compare with the baseline, store everything, return the report's markdown."""
    sid = ctx["id"]
    base_branch = ctx["base_ref"] if pr else ctx["ref"]
    base = None
    if store:
        raw = store.read(f"baseline/{sid}/{base_branch}.json")
        base = json.loads(raw)["shots"] if raw else None
    for s in screens:
        b = (base or {}).get(s["name"])
        s["png_path"] = store.put_object(s["png"], "png") if store else None
        if store:
            s["txt_path"] = store.put_object(s["text"].encode(), "txt")
            s["json_path"] = store.put_object(s["json"], "json")
        if not b:
            s["status"] = "new"
        elif b["png"] == s["png_path"]:
            s["status"] = "unchanged"
        else:
            s["status"] = "changed"
            s["base_png"] = b["png"]
            s["base_text"] = (store.read(b["txt"]) or b"").decode("utf-8", "replace")
            base_json = store.read(b["json"])
            if base_json:
                try:
                    png, s["diff_desc"] = renderer.diff_png(s, base_json)
                    s["diff_path"] = store.put_object(png, "png")
                except (RuntimeError, ValueError, KeyError) as e:
                    log(f"::warning::no diff image for {s['name']}: {e}")

    published = False
    if store:
        entry = lambda s: {k: s[k + "_path"] for k in ("png", "txt", "json")} | (
            {"diff": s["diff_path"], "base_png": s["base_png"]} if s.get("diff_path") else {})
        manifest = {"termshot": renderer.version, "commit": ctx["sha"],
                    "updated": now().isoformat(), "shots": {s["name"]: entry(s) for s in screens}}
        if pr:
            manifest["base_ref"] = ctx["base_ref"]
            store.put(f"pr/{sid}/{pr['number']}.json", json.dumps(manifest, indent=1).encode())
        else:
            store.put(f"baseline/{sid}/{base_branch}.json", json.dumps(manifest, indent=1).encode())
        n = len(store.staged)
        store.commit(f"termshot {sid}: {'#%d' % pr['number'] if pr else base_branch} "
                     f"at {ctx['sha'][:12]}")
        log(f"stored {n} files on {store.branch}")
        published = True
        if not pr:
            prune(gh, store, int(inp("retention-days", "30")))

    url = (lambda p: raw_url(store.gh.repo, store.branch, p, ctx["private"]) if p else None) \
        if published else (lambda p: None)
    return report.report(ctx, screens, set(base) if base is not None else None, url)


def prune(gh, store, days):
    """Drop the manifests of pull requests closed more than `days` ago, and of deleted
    branches, then every object no manifest names. History is squashed only if
    something went, which is what frees the space."""
    if days <= 0 or store.truncated:
        return
    cutoff = now() - datetime.timedelta(days=days)
    keep, dropped = set(), []
    for path in store.tree:
        m = re.fullmatch(r"pr/[^/]+/(\d+)\.json", path)
        if m:
            p = gh.call("GET", f"/pulls/{m[1]}", ok404=True)
            closed = p and p["state"] == "closed" and p["closed_at"] and \
                datetime.datetime.fromisoformat(p["closed_at"].replace("Z", "+00:00")) < cutoff
            if closed or p is None:
                dropped.append(path)
            else:
                keep.add(path)
            continue
        m = re.fullmatch(r"baseline/[^/]+/(.+)\.json", path)
        if m:
            gone = gh.call("GET", f"/branches/{quote(m[1], safe='')}", ok404=True) is None
            updated = json.loads(store.read(path)).get("updated", "")
            old = not updated or datetime.datetime.fromisoformat(updated) < cutoff
            if gone and old:
                dropped.append(path)
            else:
                keep.add(path)
    for path in list(keep):
        for e in json.loads(store.read(path))["shots"].values():
            keep.update(v for v in e.values() if v in store.tree)
    if keep == set(store.tree):
        return
    removed = len(store.tree) - len(keep)
    if store.replace(keep, f"termshot: prune {removed} files"):
        log(f"pruned {removed} files ({len(dropped)} manifests) from {store.branch}")


# ---------------------------------------------------------------- modes


def payload():
    path = os.environ.get("GITHUB_EVENT_PATH")
    if path and os.path.exists(path):
        with open(path) as f:
            return json.load(f)
    return {}


def make_ctx(renderer, sha, base_ref, private):
    repo = os.environ.get("GITHUB_REPOSITORY", "")
    run = os.environ.get("GITHUB_RUN_ID")
    return {"id": inp("id", "termshot"), "version": renderer.version, "sha": sha,
            "base_ref": base_ref, "ref": os.environ.get("GITHUB_REF_NAME", ""),
            "private": private, "run_url": f"{SERVER}/{repo}/actions/runs/{run}" if run else ""}


def finish(ctx, screens, renderer, pr, can_write):
    """Compare, publish and comment when allowed. Returns whether a comment was posted."""
    repo = os.environ.get("GITHUB_REPOSITORY", "")
    token = inp("github-token")
    dry = bool(os.environ.get("TERMSHOT_DRY_RUN"))
    gh = GitHub(token, repo) if token and repo and can_write and not dry else None
    store = None
    if gh and inp("publish", "true") != "false":
        assets_repo = inp("assets-repo", repo)
        try:
            store = Store(GitHub(inp("assets-token", token), assets_repo),
                          inp("assets-branch", "termshot-assets"))
        except HTTPError as e:
            log(f"::warning::cannot read {assets_repo}: {e}")
    try:
        body = compare_and_publish(ctx, screens, renderer, gh, store, pr)
    except HTTPError as e:
        if e.code not in (403, 404):
            raise
        log(f"::warning::could not store the screens ({e}). Give the job "
            "`permissions: contents: write`, or see the README on forks.")
        for s in screens:
            s.setdefault("status", "new")
        body = report.report(ctx, screens, None, lambda p: None)
        store = None
    summary(body)
    commented = False
    if pr and gh and inp("comment", "auto") != "never":
        try:
            c = gh.upsert_comment(pr["number"], report.MARK.format(id=ctx["id"]), body)
            log(f"comment: {c['html_url']}")
            set_output("comment-url", c["html_url"])
            commented = True
        except HTTPError as e:
            log(f"::warning::could not comment ({e}). Give the job "
                "`permissions: pull-requests: write`.")
    n = sum(s["status"] == "changed" for s in screens)
    set_output("changed", str(n))
    return commented, n


def summary(body):
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as f:
            f.write(body + "\n")
    if os.environ.get("TERMSHOT_DRY_RUN"):
        print(body)


def mode_run(renderer, out_dir):
    default_size = capture.parse_size(inp("size", "100x30"), None)
    screens = gather(renderer, default_size, float(inp("timeout", "10")))
    check_names(screens)
    for s in screens:
        renderer.render(s)
    ev = payload()
    pr = ev.get("pull_request")
    private = (ev.get("repository") or {}).get("private", True)
    repo = os.environ.get("GITHUB_REPOSITORY", "")
    fork = bool(pr) and pr["head"]["repo"]["full_name"] != repo
    bot = os.environ.get("GITHUB_ACTOR") == "dependabot[bot]"
    ctx = make_ctx(renderer, pr["head"]["sha"] if pr else os.environ.get("GITHUB_SHA", ""),
                   pr["base"]["ref"] if pr else "", private)
    if fork or bot:
        log("::notice::the token here is read-only (" + ("a fork" if fork else "Dependabot")
            + "), so the screens go to the artifact. A workflow_run workflow with this "
            "action can publish them; see the README.")
        for s in screens:
            s["status"] = "new"
        summary(report.report(ctx, screens, None, lambda p: None))
        commented, n = False, 0
    else:
        commented, n = finish(ctx, screens, renderer, pr, can_write=True)
    bundle = {"format": 1, "id": ctx["id"], "published": commented or not pr,
              "pr": pr["number"] if pr else None, "head_sha": ctx["sha"],
              "screens": [{"name": s["name"], "label": s["label"], "size": "%dx%d" % s["size"],
                           "note": s["note"], "steps": s["steps"], "exit": s["exit"]}
                          for s in screens]}
    with open(os.path.join(out_dir, BUNDLE), "w") as f:
        json.dump(bundle, f, indent=1)
    return n


def mode_publish(renderer):
    run = payload().get("workflow_run")
    if not run:
        fail("mode publish runs on a workflow_run event")
    if run["event"] != "pull_request":
        log(f"::notice::nothing to publish for a {run['event']} run")
        return 0
    repo = os.environ["GITHUB_REPOSITORY"]
    gh = GitHub(inp("github-token"), repo)
    sid = inp("id", "termshot")
    files = gh.artifact(run["id"], f"termshot-{sid}", ARTIFACT_LIMIT)
    if files is None:
        log(f"::notice::run {run['id']} left no termshot-{sid} artifact")
        return 0
    bundle = json.loads(files[BUNDLE])
    if bundle.get("published"):
        log("::notice::that run already published its screens")
        return 0
    # Everything below comes from code we did not review: check it all.
    if bundle.get("format") != 1 or bundle.get("id") != sid or not isinstance(bundle.get("pr"), int):
        fail("the artifact's bundle is not one this action wrote")
    pr = gh.call("GET", f"/pulls/{bundle['pr']}", ok404=True)
    if not pr or pr["head"]["sha"] != run["head_sha"] or \
            pr["head"]["repo"]["full_name"] != (run.get("head_repository") or {}).get("full_name"):
        fail(f"pull request #{bundle['pr']} is not the one run {run['id']} was for")
    if pr["state"] != "open":
        log("::notice::the pull request is closed")
        return 0
    items = bundle.get("screens")
    if not isinstance(items, list) or not 0 < len(items) <= MAX_SCREENS:
        fail("the bundle has no screens, or too many")
    screens = []
    for it in items:
        name = it.get("name") if isinstance(it, dict) else None
        if not isinstance(name, str) or not capture.NAME_RE.match(name) or len(name) > 100:
            fail(f"bad screen name {name!r}")
        size = capture.parse_size(str(it.get("size")), None)
        logf = files.get(f"{name}.pty")
        if logf is None or len(logf) > 16 << 20:
            fail(f"no log, or too big a log, for {name}")
        ex = it.get("exit")
        screens.append({
            "name": name, "size": size, "log": logf,
            "label": str(it.get("label", ""))[:500], "note": str(it.get("note", ""))[:300],
            "steps": it["steps"] if isinstance(it.get("steps"), int) else 0,
            "exit": ex if isinstance(ex, int) and not isinstance(ex, bool) else None})
    check_names(screens)
    for s in screens:
        renderer.render(s)
    private = pr["base"]["repo"]["private"]
    ctx = make_ctx(renderer, pr["head"]["sha"], pr["base"]["ref"], private)
    ctx["run_url"] = run.get("html_url", "")
    _, n = finish(ctx, screens, renderer, pr, can_write=True)
    return n


def main():
    out_dir = os.path.abspath(inp("output-dir", "termshot-out"))
    os.makedirs(out_dir, exist_ok=True)
    set_output("dir", out_dir)
    sid = inp("id", "termshot")
    if not capture.NAME_RE.match(sid):
        fail(f"id must match {capture.NAME_RE.pattern}")
    renderer = Renderer(inp("termshot", "termshot"), out_dir)
    mode = inp("mode", "auto")
    if mode == "auto":
        mode = "publish" if os.environ.get("GITHUB_EVENT_NAME") == "workflow_run" else "run"
    try:
        n = mode_publish(renderer) if mode == "publish" else mode_run(renderer, out_dir)
    except (capture.SpecError, RuntimeError) as e:
        fail(str(e))
    if n and inp("fail-on-change") == "true":
        fail(f"{n} screen{'s' * (n != 1)} changed")


if __name__ == "__main__":
    main()
