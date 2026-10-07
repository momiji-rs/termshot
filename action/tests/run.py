#!/usr/bin/env python3
"""Tests for the action against an in-memory GitHub: python3 action/tests/run.py [termshot]

Covers capture with steps and sizes, baselines, compare and diff, the sticky comment,
the fork path through an artifact and mode publish, the checks on that artifact,
and pruning.
"""

import io
import json
import os
import subprocess
import sys
import tempfile
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
ACTION = os.path.join(os.path.dirname(HERE), "termshot_action.py")
sys.path.insert(0, HERE)
import fake_github  # noqa: E402

TERMSHOT = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else
                           os.path.join(HERE, "..", "..", "termshot"))
REPO = "o/r"
failures = []


def check(cond, what):
    print(("ok   " if cond else "FAIL ") + what)
    if not cond:
        failures.append(what)


def run(env, event_name, event, tmp, expect=0):
    ev = os.path.join(tmp, f"event-{len(os.listdir(tmp))}.json")
    with open(ev, "w") as f:
        json.dump(event, f)
    out = tempfile.mkdtemp(dir=tmp)
    full = dict(os.environ, GITHUB_API_URL=API, GITHUB_REPOSITORY=REPO, GITHUB_RUN_ID="9",
                GITHUB_EVENT_NAME=event_name, GITHUB_EVENT_PATH=ev, INPUT_GITHUB_TOKEN="t",
                INPUT_TERMSHOT=TERMSHOT, INPUT_OUTPUT_DIR=out, INPUT_SIZE="40x6",
                INPUT_PX="16", GITHUB_OUTPUT=os.path.join(out, "outputs"), **env)
    r = subprocess.run([sys.executable, ACTION], env=full, capture_output=True, text=True)
    if r.returncode != expect:
        print(r.stdout, r.stderr, sep="\n")
    check(r.returncode == expect, f"exit {expect}: {event_name} {sorted(env)}")
    outputs = {}
    if os.path.exists(full["GITHUB_OUTPUT"]):
        for line in open(full["GITHUB_OUTPUT"]):
            k, _, v = line.rstrip("\n").partition("=")
            outputs[k] = v
    return out, outputs, r


def push(branch, sha, shots, tmp, **env):
    return run(dict(GITHUB_REF_NAME=branch, GITHUB_SHA=sha, INPUT_SHOTS=shots, **env),
               "push", {"repository": {"private": False}}, tmp)


def pr_event(number, sha, head_repo=REPO, base="main"):
    pr = {"number": number, "state": "open", "closed_at": None,
          "head": {"sha": sha, "repo": {"full_name": head_repo}},
          "base": {"ref": base, "repo": {"private": False}}}
    S.pulls[number] = pr
    return {"repository": {"private": False}, "pull_request": pr}


def comment(number):
    cs = [c for c in S.comments if c["pr"] == number]
    return cs[-1]["body"] if cs else "", len(cs)


GREET = 'greet: printf "\\e[1;32mhello\\e[0m world\\r\\n"\nsame: echo unchanged\ngone: echo bye'
GREET2 = 'greet: printf "\\e[1;35mhello\\e[0m there\\r\\n"\nsame: echo unchanged\nfresh: echo new'


def main():
    tmp = tempfile.mkdtemp()

    print("== capture: steps, snaps, sizes")
    shots = ("pager@30x5: seq 1 100 | less\n"
             "  wait-for 1\n"
             "  snap top\n"
             "  key space\n"
             "  snap next\n"
             "  key q\n"
             "ask: read -p 'name? ' n; echo \"hi $n\"; sleep 30\n"
             "  wait-for name?\n"
             "  type Ada\n"
             "  key enter\n"
             "  wait-for hi Ada\n"
             "tiny@12x2: echo small\n")
    out, _, r = run({"INPUT_SHOTS": shots, "TERMSHOT_DRY_RUN": "1"}, "push", {}, tmp)
    text = lambda n: open(os.path.join(out, n + ".txt")).read()
    check(text("pager.top").splitlines()[0] == "1", "snap top shows line 1")
    check(text("pager.next").splitlines()[0] == "5", "key space pages on (30x5: lines 5..)")
    check("exit 0" in r.stderr.split("captured pager:")[1].splitlines()[0], "q quits less")
    check("hi Ada" in text("ask"), "type and enter reach the program")
    check("captured ask: running" in r.stderr, "a program still running is captured")
    check(json.load(open(os.path.join(out, "tiny.json")))["cols"] == 12, "per-shot size")
    _, _, r = run({"INPUT_SHOTS": "x: true\n  key nosuchkey", "TERMSHOT_DRY_RUN": "1"},
                  "push", {}, tmp, expect=1)
    check("unknown key" in r.stdout, "a bad step is refused with its line")

    print("== baseline, compare, diff, sticky comment")
    push("main", "1" * 40, GREET, tmp)
    check("baseline/termshot/main.json" in S.files(), "push stores a baseline")
    out, o, _ = run({"INPUT_SHOTS": GREET2}, "pull_request", pr_event(7, "a" * 40), tmp)
    body, n = comment(7)
    check(o.get("changed") == "1", "one screen changed")
    check("1 changed" in body and "1 new" in body and "1 removed" in body, "counts")
    check("greet.diff" not in body and "diff** · 10 cells changed, in row 1" in body,
          "cell diff is described")
    check("-hello world" in body and "+hello there" in body, "text diff")
    check("raw.githubusercontent.com/o/r/termshot-assets/objects/" in body, "content-addressed URLs")
    check(os.path.exists(os.path.join(out, "greet.diff.png")), "diff image rendered")
    run({"INPUT_SHOTS": GREET2, "INPUT_FAIL_ON_CHANGE": "true"}, "pull_request",
        pr_event(7, "b" * 40), tmp, expect=1)
    check(comment(7)[1] == 1, "the comment is updated, not repeated")
    objects = [p for p in S.files() if p.startswith("objects/")]
    run({"INPUT_SHOTS": GREET2}, "pull_request", pr_event(7, "c" * 40), tmp)
    check(len([p for p in S.files() if p.startswith("objects/")]) == len(objects),
          "the same screens store no new objects")

    print("== a fork: render only, then mode publish")
    fork_ev = pr_event(8, "f" * 40, head_repo="someone/r")
    calls = len(S.calls)
    out, _, r = run({"INPUT_SHOTS": GREET2}, "pull_request", fork_ev, tmp)
    check(len(S.calls) == calls, "a fork run makes no API calls")
    bundle = json.load(open(os.path.join(out, ".termshot-bundle.json")))
    check(bundle["pr"] == 8 and not bundle["published"], "the bundle names the PR, unpublished")

    def artifact(run_id, files):
        buf = io.BytesIO()
        with zipfile.ZipFile(buf, "w") as z:
            for name, data in files.items():
                z.writestr(name, data)
        S.artifacts[run_id] = [("termshot-termshot", buf.getvalue())]

    files = {n: open(os.path.join(out, n), "rb").read() for n in os.listdir(out)
             if n.endswith(".pty") or n == ".termshot-bundle.json"}
    artifact(50, files)
    wr = lambda run_id, sha, repo="someone/r": {"workflow_run": {
        "id": run_id, "event": "pull_request", "head_sha": sha, "html_url": "https://x/run",
        "head_repository": {"full_name": repo}}}
    run({}, "workflow_run", wr(50, "f" * 40), tmp)
    body, n = comment(8)
    check(n == 1 and "1 changed" in body and "+hello there" in body, "publish comments on the fork PR")
    check(any(m == "GET" and p.startswith("/blobstore/") for m, p in S.calls),
          "the artifact is fetched from storage without the token")

    print("== mode publish refuses what doesn't match")
    _, _, r = run({}, "workflow_run", wr(50, "e" * 40), tmp, expect=1)
    check("is not the one run" in r.stdout, "a head sha that isn't the PR's")
    _, _, r = run({}, "workflow_run", wr(50, "f" * 40, repo="other/r"), tmp, expect=1)
    check("is not the one run" in r.stdout, "a head repository that isn't the PR's")
    bad = dict(files)
    b = json.loads(bad[".termshot-bundle.json"])
    b["screens"][0]["name"] = "../../etc/x"
    bad[".termshot-bundle.json"] = json.dumps(b)
    artifact(51, bad)
    _, _, r = run({}, "workflow_run", wr(51, "f" * 40), tmp, expect=1)
    check("bad screen name" in r.stdout, "a screen name with a path in it")
    b["screens"][0]["name"] = "greet"
    b["screens"][0]["size"] = "9999x9999"
    bad[".termshot-bundle.json"] = json.dumps(b)
    artifact(52, bad)
    _, _, r = run({}, "workflow_run", wr(52, "f" * 40), tmp, expect=1)
    check("size must be" in r.stdout, "a size over the limit")
    b["published"] = True
    bad[".termshot-bundle.json"] = json.dumps(b)
    artifact(53, bad)
    _, _, r = run({}, "workflow_run", wr(53, "f" * 40), tmp)
    check("already published" in r.stderr, "a run that published itself is left alone")
    S.pulls[8]["state"] = "closed"
    _, _, r = run({}, "workflow_run", wr(50, "f" * 40), tmp)
    check("closed" in r.stderr, "a closed PR is left alone")

    print("== pruning")
    S.pulls[7].update(state="closed", closed_at="2020-01-01T00:00:00Z")
    S.pulls[8].update(state="closed", closed_at="2099-01-01T00:00:00Z")
    pr8 = {p for p in S.files() if p == "pr/termshot/8.json"}
    before = set(S.files())
    push("main", "2" * 40, GREET, tmp)
    after = set(S.files())
    check("pr/termshot/7.json" not in after, "an old closed PR's manifest goes")
    check(pr8 <= after, "a recently closed PR's manifest stays")
    check(len(after) < len(before), "its unshared objects go")
    head = S.commits[S.refs["termshot-assets"]]
    check(head["parents"] == [], "history is squashed so the space is freed")
    for path, data in S.files().items():
        if path.endswith(".json") and not path.startswith("objects/"):
            for e in json.loads(data)["shots"].values():
                check(all(v in after for v in e.values()), f"everything {path} names is kept")

    print(f"\n{len(failures)} failed" if failures else "\nall passed")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    S = fake_github.State()
    srv = fake_github.serve(S)
    API = f"http://127.0.0.1:{srv.server_address[1]}"
    main()
