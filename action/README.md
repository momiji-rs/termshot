# termshot action (proof of concept)

Screenshot your CLI or TUI in CI, and see on the pull request what changed.

```yaml
# .github/workflows/screens.yml
name: screens
on:
  pull_request:
  push:
    branches: [main]

permissions:
  contents: write        # store the PNGs on the termshot-assets branch
  pull-requests: write   # post the comment

jobs:
  screens:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: cargo build --release   # or whatever builds your program
      - uses: momiji-rs/termshot/action@main
        with:
          shots: |
            help: ./target/release/myapp --help
            list@120x20: ./target/release/myapp list --color=always
            inbox: ./target/release/myapp
              wait-for Inbox
              key down down
              snap selected
              type hello
              key enter
```

Each pull request gets **one comment, updated in place on every push**:

- **changed** screens show before and after side by side, a **diff image** (the new screen with
  every changed cell at full strength and the rest dimmed, so a colour-only change shows too),
  and a diff of their text
- **new** screens are shown in full, and **removed** ones are listed
- **unchanged** screens fold into one line

## Shots

A shot is `name: command`, or `name@COLSxROWS: command` for a size other than `size`. The
command runs under bash in a real PTY of that size, with `TERM=xterm-256color`, so programs
colour and lay out their output as they would for a person.

With no steps, the screen is taken when the command exits, or at `timeout` if it is still running
(a TUI). Indented lines below a shot drive it instead:

| step | |
|---|---|
| `wait 0.5` | let it run for that many seconds |
| `wait-for TEXT` | until TEXT is on the screen (termshot reads the screen as it goes) |
| `type TEXT` | type TEXT |
| `key NAME …` | press keys: `enter`, `tab`, `esc`, `space`, `backspace`, `up`, `down`, `left`, `right`, `home`, `end`, `pgup`, `pgdn`, `delete`, `f1`–`f12`, `shift-tab`, `ctrl-c`, `alt-x`, or one character |
| `snap NAME` | take a screen now, as `<shot>.NAME` |

After the last step the final screen is taken as `<shot>`. A program still running then is
killed after recording has stopped, so its exit cleanup (leaving the alternate screen) never
reaches the image.

`logs: 'tests/screens/*.pty@100x30'` takes PTY logs you recorded yourself instead.

## How it works

1. **Render.** termshot turns each log into a PNG, its `--text` and its `--json`. It needs no
   browser, ffmpeg or font setup: about 10 ms a screen, from a static binary checked against the
   release's SHA256SUMS.
2. **Compare.** termshot is deterministic, so the same screen gives the same bytes on every runner,
   and a change is exact rather than a pixel-tolerance guess. A push to a branch stores that
   branch's screens as its baseline. A pull request compares with the baseline of its base branch.
   The diff is per cell, from `--json` (character, colours, attributes), and termshot draws it:
   the action writes a log of the new screen with the changed cells lit and renders that.
3. **Store.** GitHub has no API for uploading images to comments, so the files are committed to an
   orphan branch, `termshot-assets`, which never touches your history. It is content-addressed:
   each file is `objects/<sha256>` and is never rewritten, so a screen that didn't change is not
   stored again and old comments keep their images.
4. **Prune.** A push removes what pull requests closed more than `retention-days` ago (30) used
   alone, and the baselines of deleted branches, and squashes the branch's history so the space
   is freed. Comments on those pull requests then lose their images.

Set `fail-on-change: true` to make a changed screen fail the check, like a snapshot test. The
report also goes to the job summary, and every log, PNG and text to the `termshot-<id>` artifact.

## Permissions

The action stores images with the workflow's own `GITHUB_TOKEN`. It needs no secret, no app and
no service outside GitHub. What that token may do depends on the event:

| event | token | what the action does |
|---|---|---|
| `push` | what `permissions:` grants | stores the branch's baseline, prunes |
| `pull_request` from a branch of the repository | what `permissions:` grants | compares, stores, comments |
| `pull_request` from a fork | **read-only, whatever `permissions:` says**, and no secrets | renders only; leaves the logs in the artifact |
| `pull_request` by Dependabot | read-only | as for a fork |
| `workflow_run` (after any of the above) | what `permissions:` grants, in the base repository | `mode: publish`: publishes what a fork run left |

`permissions:` can only lower the token to what the repository allows. Since 2023, new
repositories and organizations default to read-only `GITHUB_TOKEN`s, but a workflow can still
ask for `contents: write` and `pull-requests: write` unless an organization or enterprise policy
forbids it.

### Pull requests from forks

GitHub gives a fork's run a read-only token on purpose: that run executes the fork's code. The
action never asks for more there. Add a second workflow, which runs from your default branch with
a write token but never runs the fork's code:

```yaml
# .github/workflows/screens-publish.yml
name: screens-publish
on:
  workflow_run:
    workflows: [screens]
    types: [completed]

permissions:
  actions: read          # download the other run's artifact
  contents: write
  pull-requests: write

jobs:
  publish:
    if: github.event.workflow_run.event == 'pull_request'
    runs-on: ubuntu-latest
    steps:
      - uses: momiji-rs/termshot/action@main   # mode publish, from the event
```

It takes the logs from the artifact as untrusted data:

- It reads the zip in memory, under a size limit, and never unpacks it to disk.
- It checks every screen name and size.
- It looks up the pull request through the API and requires its head commit and repository to be
  the run's.
- It renders the logs with its own termshot, built to read hostile input, and doesn't trust the
  images the fork's run made.
- It runs nothing from the artifact, and writes no baseline.

A run that could publish by itself marks its artifact as published, so the second workflow does
nothing then.

Don't use `pull_request_target` for this: it has a write token too, and checking out the fork's
code under it is how repositories get compromised.

### Settings that can get in the way

- **Allowed actions.** An organization that allows only selected actions has to allow
  `momiji-rs/termshot*`.
- **Rulesets and branch protection.** A ruleset that targets all branches can stop
  `GITHUB_TOKEN` from creating `termshot-assets` or force-pushing it when pruning. Exclude the
  branch from the ruleset, or set `retention-days: 0`.
- **Private repositories.** Image URLs are `github.com/<repo>/raw/termshot-assets/…`, which only
  a signed-in viewer with access can open. This has not been verified yet. A fork of a private
  repository gets a write token only if the repository allows it ("Send write tokens to workflows
  from pull requests").
- **Keeping images out of the repository.** Set `assets-repo` to another repository you own, and
  `assets-token` to a fine-grained token or GitHub App token with `contents: write` there. Secrets
  are not given to fork runs, but they are given to `workflow_run`, so this works with the
  publish workflow. A public assets repository makes a private project's screens public.
- Pushes made with `GITHUB_TOKEN` don't start other workflows, so storing screens can't loop.

## Inputs

| input | default | |
|---|---|---|
| `shots` | | shots and their steps, as above |
| `logs` | | globs of PTY logs already recorded, each optionally `@COLSxROWS` |
| `size` | `100x30` | terminal size for shots and logs that don't give one |
| `px` | `28` | font pixel height |
| `timeout` | `10` | seconds a shot may take |
| `font`, `fallback-font`, `args` | | passed to termshot; give the publish workflow the same |
| `mode` | `auto` | `run`, or `publish` (the default on `workflow_run`) |
| `id` | `termshot` | names this set, for several uses in one repository |
| `comment` | `auto` | `never` to skip the comment |
| `publish` | `true` | `false` to store nothing (the comment then has text only) |
| `assets-branch` | `termshot-assets` | |
| `assets-repo`, `assets-token` | this repository, `github-token` | where to store the files |
| `retention-days` | `30` | `0` never prunes |
| `fail-on-change` | `false` | |
| `termshot-version` / `termshot-path` | `0.2.0` | release to download, or a binary to use |

Outputs: `changed` (count), `dir` (logs, PNGs and texts), `comment-url`.

## Tests

`python3 action/tests/run.py ./termshot` runs the action against an in-memory GitHub API:
capture with steps and sizes, baselines, compare and diff, the sticky comment, the fork path
through an artifact, the checks on that artifact, and pruning.

## Limits

- Linux and macOS runners only; Windows has no PTY that Python can open.
- A matrix job needs a different `id` per leg, or the artifacts clash.
- Pruning while another job stores screens can, rarely, drop an object that job reused; its next
  push stores it again.
