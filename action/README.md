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
  contents: write        # push the PNGs to the termshot-assets branch
  pull-requests: write   # post the comment

jobs:
  screens:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: cargo build --release   # or whatever builds your program
      - uses: momiji-rs/termshot/action@main
        with:
          size: 100x30
          shots: |
            help: ./target/release/myapp --help
            list: ./target/release/myapp list --color=always
            tui:  ./target/release/myapp      # still running at timeout: its screen is taken
```

Each pull request gets **one comment, updated in place on every push**:

- **changed** screens show before and after side by side, plus a diff of their text
- **new** screens are shown in full, and **removed** ones are listed
- **unchanged** screens fold into one line

## How it works

1. **Capture.** Each command runs under bash in a real PTY of `size` (with `TERM=xterm-256color`),
   so programs colour and lay out their output as they would for a person. A command still running
   after `timeout` seconds is a TUI showing its screen. Recording stops first and the program is
   killed after, so its exit cleanup (leaving the alternate screen) never reaches the image. You
   can also pass logs you recorded yourself with `logs: 'tests/screens/*.pty'`.
2. **Render.** termshot turns each log into a PNG and its `--text`. It needs no browser, ffmpeg
   or font setup: about 10 ms a screen, from a static binary checked against the release's
   SHA256SUMS.
3. **Compare.** termshot is deterministic, so the same screen gives the same bytes on every runner,
   and a change is exact rather than a pixel-tolerance guess. A push to a branch stores that
   branch's screens as its baseline. A pull request compares with the baseline of its base branch.
4. **Publish.** GitHub has no API for uploading images to comments, so the PNGs are committed to an
   orphan branch, `termshot-assets`, which never touches your history. Image URLs are pinned to a
   commit, so old comments keep showing what they showed. The same report goes to the job summary,
   and the files go to an artifact.

Set `fail-on-change: true` to make a changed screen fail the check, like a snapshot test.

## Inputs

| input | default | |
|---|---|---|
| `shots` | | `name: command` per line |
| `logs` | | globs of PTY logs already recorded |
| `size` | `100x30` | terminal size |
| `px` | `28` | font pixel height |
| `timeout` | `10` | seconds before a still-running command's screen is taken |
| `font`, `fallback-font`, `args` | | passed to termshot |
| `id` | `termshot` | names this set, for several uses in one repository |
| `comment` | `auto` | `never` to skip the comment |
| `publish` | `true` | `false` to keep images out of the assets branch (the comment then has text only) |
| `assets-branch` | `termshot-assets` | |
| `fail-on-change` | `false` | |
| `termshot-version` / `termshot-path` | `0.2.0` | release to download, or a binary to use |

Outputs: `changed` (count), `dir` (logs, PNGs and texts), `comment-url`.

## Limits of this proof of concept

- **Forks.** A pull request from a fork gets a read-only token, so it can't push or comment.
  The action then reports to the job summary and the artifact. The safe fix is the two-workflow
  pattern: render without permissions, then publish from a `workflow_run` workflow that treats the
  artifact as data. Not done yet.
- **Private repositories.** Images use `github.com/<repo>/raw/<sha>/…`, which only a signed-in
  viewer with access can open. This has not been tested.
- **The assets branch only grows.** It should prune pull request folders after a merge, or after
  some days.
- Linux and macOS runners only; Windows has no PTY that Python can open.
