# termshot

[![ci](https://github.com/momiji-rs/termshot/actions/workflows/ci.yml/badge.svg)](https://github.com/momiji-rs/termshot/actions/workflows/ci.yml)
[![release](https://img.shields.io/github/v/release/momiji-rs/termshot)](https://github.com/momiji-rs/termshot/releases/latest)
[![MSRV 1.70](https://img.shields.io/badge/rustc-1.70%2B-orange)](CONTRIBUTING.md)
[![license MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

**A screenshot tool for terminal output you already have.** termshot replays a raw PTY log
or an asciinema recording, rebuilds the screen, and writes a PNG of the final frame. It
runs headless, with no terminal or window, and gives the same pixels on macOS and Linux.

![A 100 by 30 demo inbox, rendered from examples/reply-sent.pty](docs/reply-sent.png)

<sub>`examples/reply-sent.pty`, a capture of the [crisp-tui](https://github.com/solcreek/crisp-tui)
demo inbox (the customers and messages are fake), at the default 48 px: 2200×1440. `examples/draft-ready.pty` is another.</sub>

- **No dependencies.** One static binary with its font built in. It builds from source with
  `rustc` and a C compiler, and needs no Cargo or crates.io.
- **Deterministic.** Same bytes in, same PNG out, on macOS and on x86-64 and aarch64
  Linux. Commit a reference image and compare against it in CI.
- **Readable by tests and agents.** `--text` and `--json` write the same screen as text, or as
  runs of cells with their colours and attributes.
- **Embeddable.** It is also a Rust library: parse a log into a grid, render the grid to a PNG.

## Why termshot

We built it for TUI work, where what matters is what the screen ends up showing:

- **Developing a TUI.** Record what the app writes, render it, and look at the frame without
  opening a terminal. This works on a remote box or inside an agent loop too.
- **Verifying output.** Catch layout, colour and box-drawing regressions by comparing a
  frame against a committed image or a `--text` snapshot.
- **Screenshots for docs, READMEs and PRs.** A crisp image at any pixel height, with no
  dependence on your terminal's theme, font or window size.
- **Bug reports.** Ask for the raw log, not a phone photo of the screen, and render it on your
  end.

termshot is narrow on purpose. Other tools do other jobs well:

| tool | input | output | pick it for |
| --- | --- | --- | --- |
| **termshot** | bytes a terminal already got: a PTY log, `tmux capture-pane`, a `.cast` | PNG of the final frame; text; JSON | rendering existing output, headless and reproducibly; tests; embedding in Rust |
| [vhs](https://github.com/charmbracelet/vhs) | a `.tape` script that it types into a shell | GIF, MP4, WebM, PNG | scripted demo recordings and animations |
| [freeze](https://github.com/charmbracelet/freeze) | code, or a command's ANSI output | PNG, SVG, WebP | styled images of code or output, with window chrome and shadows |
| [homeport/termshot](https://github.com/homeport/termshot) | a command it runs | PNG | a styled screenshot of one command's output |
| [agg](https://github.com/asciinema/agg) | a `.cast` | GIF | animating an asciinema recording |

termshot draws one frame, with no animation and no window chrome.

## Quick start

Download a release archive and put the binary on your `PATH`:

```sh
v=0.3.1 p=linux-x86_64-musl   # or linux-aarch64-musl, macos-universal
curl -LO https://github.com/momiji-rs/termshot/releases/download/v$v/termshot-$v-$p.tar.gz
tar -xzf termshot-$v-$p.tar.gz
export PATH="$PWD/termshot-$v-$p:$PATH"
termshot --version
```

Then render some output:

```sh
printf '\033[1;32mok\033[0m build passed in 2.1s\r\n' | termshot --size 40x2 - hello.png
```

For a real session, record it through a PTY (with `asciinema rec`, `script`, or tmux; see
[Usage](#usage)) and give `--size` the terminal size it used. The default is 100×30.

## Install

[GitHub Releases](https://github.com/momiji-rs/termshot/releases) has an archive for each
platform, with `SHA256SUMS`:

| archive | runs on |
| --- | --- |
| `termshot-0.3.1-macos-universal.tar.gz` | macOS 11 or newer, arm64 and x86_64 |
| `termshot-0.3.1-linux-x86_64-musl.tar.gz` | x86_64 Linux, static (no libc needed) |
| `termshot-0.3.1-linux-aarch64-musl.tar.gz` | aarch64 Linux, static (no libc needed) |

Each archive holds `termshot-0.3.1-<platform>/`, with the binary, this README, the changelog
and the licenses. The binary carries its font, so it needs no other files. The archives hold
the CLI only; for the library, [build from source](#use-as-a-rust-library).

```sh
sha256sum -c --ignore-missing SHA256SUMS    # or: shasum -a 256 -c --ignore-missing SHA256SUMS
```

**From npm**, on the same platforms, with Node 22 or newer. The package runs the release
binary, which npm installs for your platform as an optional dependency:

```sh
npx -y @momiji-rs/termshot session.pty session.png
npm install -g @momiji-rs/termshot
```

**From source**, with a C compiler and rustc 1.70 or newer:

```sh
./build.sh      # writes ./termshot and libtermshot.rlib
```

## Usage

```sh
termshot session.pty session.png                            # 100×30 at 48 px
termshot --size 120x40 --px 24 session.pty session.png
termshot --font MyFont.ttf --fallback-font NotoSansCJK.ttc#3 session.pty session.png
cat session.pty | termshot - - > screen.png                # stdin to stdout
```

**A TUI that is still running**: drive it in tmux and capture the pane. tmux ends each row
with a bare LF, so pass `--lf-newline`, and pass the cursor, which the capture leaves out:

```sh
tmux new-session -d -s app -x 100 -y 30 top
cursor=$(tmux display -p -t app '#{?cursor_flag,#{cursor_x}#,#{cursor_y},none}')
tmux capture-pane -t app -e -p | termshot --lf-newline --size 100x30 --cursor "$cursor" - top.png
```

**An asciinema recording** (asciicast v2 or v3) takes its size from the file:

```sh
asciinema rec demo.cast
termshot demo.cast demo.png
```

**Text or JSON instead of pixels**, to check what a screen shows rather than how it looks.
Without a PNG, termshot doesn't load a font (unless the log has images):

```sh
termshot --text - session.pty | grep -q 'Saved'      # text only, to stdout
termshot --json screen.json session.pty screen.png   # both
```

| option | |
| --- | --- |
| `-s`, `--size CxR` | grid columns × rows, up to 500×200 (default: a cast's size, else 100x30) |
| `-p`, `--px N` | font pixel height, 1 to 255 (default 48) |
| `-f`, `--font FILE` | TrueType or OpenType font (default: built-in JetBrains Mono); `FILE#N`, `FILE#NAME` or `FILE#wght=700` pick a face or an instance |
| `--fallback-font FILE` | font for characters the first lacks, such as CJK |
| `--text FILE`, `--json FILE` | also write the screen as text or JSON; the PNG is then optional |
| `--palette FILE`, `--fg`, `--bg` | the default and 16 named colours, in kitty's theme keys |
| `--padding N` or `X,Y` | a margin around the cells, in the default background |
| `--cursor COL,ROW` or `none`, `--cursor-shape` | where and how to draw the cursor (default: as the log leaves it) |
| `--lf-newline` | treat each bare LF as CR LF, for logs not captured through a PTY |
| `--cast`, `--raw` | force the log's format |
| `-v`, `--verbose` | print the cell and image size and the faces used |

Exit status is 0 when done; 1 for a file that can't be read or written, a malformed `.cast` or
an unusable font; and 2 for bad arguments, running out of memory or an internal error. A failed
run removes the files it created.

**[docs/usage.md](docs/usage.md)** is the full reference: every option, cast parsing, the JSON
format, fonts, collections and variable fonts, palettes and exit codes.

## In CI

[termshot screens](https://github.com/marketplace/actions/termshot-screens) is a GitHub Action
built on termshot. It runs your CLI or TUI in a real PTY, can send it keys, and screenshots it. On a pull
request it posts one comment, updated on every push, with each changed screen before and
after, a diff image and a diff of the text:

```yaml
- uses: momiji-rs/termshot-action@v0
  with:
    mode: render
    shots: |
      help: ./target/release/myapp --help
      inbox: ./target/release/myapp
        wait-for Inbox
        key down down
        snap selected
```

[Its README](https://github.com/momiji-rs/termshot-action#readme) has the full setup: a
read-only job that runs your code, and a separate job that publishes the comment.

Without the Action, render in your own test job and compare `--text` output, or the PNG, with
a committed reference.

## Use as a Rust library

`build.sh` also builds `libtermshot.rlib`, with no dependencies and the C it needs bundled
inside. `termshot::parse` replays a log into a `Grid`; `termshot::render` draws it and returns
the same PNG bytes the CLI writes:

```rust
// app.rs
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let log = std::fs::read("session.pty")?;
    let grid = termshot::parse(&log, 100, 30, &termshot::ParseOptions::default())?;
    print!("{}", grid.to_text());
    let png = termshot::render(&grid, &termshot::RenderOptions::default())?;
    std::fs::write("session.png", &png.png)?;
    Ok(())
}
```

```sh
rustc --edition 2021 app.rs --extern termshot=libtermshot.rlib
```

Build your program with the same rustc that built the rlib, and pass `-L` with the rlib's
directory if it isn't the current one. Errors are values (`termshot::Error`, with the CLI's
messages): the library prints nothing and never exits, and where an allocation grows with the
input it returns `Error::OutOfMemory` instead of aborting (the crate docs list the exceptions).
`RenderOptions` takes the fonts (`Font::open`, `Font::from_bytes` or the built-in one), the pixel
size, padding and the cursor; `render_rgba` returns pixels instead of a PNG. For a log whose
images move the cursor by pixels, parse with `parse_with_cell_size` and `Font::cell_size`. Grids, fonts and renders can be shared across threads. To build the
API docs: `rustdoc --edition 2021 --crate-name termshot src/lib.rs -o target/doc`.

## What it renders

The screen model follows xterm, and `tests/vt/` checks it against tmux:

- **Screen**: autowrap, scrolling and scroll regions, line and character insert and delete,
  erase, tabs, the alternate screen (`vi`, `less`), DEC line drawing, and the cursor in block,
  underline or bar shape (DECSCUSR).
- **Colour and style**: 16, 256 and 24-bit colour; bold, dim, italic, underline, double
  underline, strike-through, reverse and hidden. Blink is not drawn.
- **Text**: wide characters (CJK, emoji) in two cells, combining marks, and box drawing and
  block elements painted as geometry, so lines join at any size. TrueType, CFF and CFF2 fonts,
  collections and variable-font instances. There is no shaping: Indic scripts are approximate,
  and emoji ZWJ sequences are not joined.
- **Images**: [kitty graphics](docs/images.md#kitty-graphics) sent inline (placements, crops,
  z-layers, Unicode placeholders) and [Sixel](docs/images.md#sixel).

A bare LF moves down without returning to column 0, as in a terminal. Logs captured through a
PTY already have CR LF; for others (`cmd > out.log`), pass `--lf-newline`.

Details are in [What it draws](docs/usage.md#what-it-draws) and
[Images in PTY logs](docs/images.md).

## Performance

Whole CLI runs, spawn to exit, median of 40, warm cache, release binaries:

| workload | image | Apple M2 Max, macOS | Ryzen 7 8745HS, Linux |
| --- | --- | ---: | ---: |
| `examples/reply-sent.pty`, 100×30 at 48 px | 2200×1440 | 8.6 ms | 8.4 ms |
| the same log at 128 px | 5800×3840 | 28.7 ms | 23.9 ms |
| a generated 240×80 screen at 48 px | 5280×3840 | 37.7 ms | 30.6 ms |
| a 4.7 MB log, 100×30 at 48 px | 2200×1440 | 18.6 ms | 17.2 ms |

Measured 2026-10-05 at `66780fc`. How long a run takes depends on the log, the image size, the
fonts and the machine. [docs/performance.md](docs/performance.md) has every round, with the
raw samples, binary hashes, toolchains, inputs and methods.

## Contributing

`./test.sh` runs everything: unit tests, C harnesses, CLI checks and the pixel goldens.
[CONTRIBUTING.md](CONTRIBUTING.md) covers building, testing, updating goldens, sanitizers and
benchmarks. [CHANGELOG.md](CHANGELOG.md) lists notable changes.

## License

termshot is MIT. Three vendored things keep their own terms:

- `third_party/stb/` is [stb](https://github.com/nothings/stb): `stb_truetype.h` 1.26,
  `stb_image_write.h` 1.16 and `stb_image.h` 2.30, public domain. `stb_image.h` decodes inline PNG images and test output, with
  only in-memory PNG decoding and bounded allocations in the binary. Local changes are listed
  in [CHANGES.md](third_party/stb/CHANGES.md).
- `third_party/jetbrains-mono/` is JetBrains Mono Regular, built into the binary,
  [SIL Open Font License 1.1](third_party/jetbrains-mono/OFL.txt).
- `third_party/noto-sans-cjk/` is a subset of Noto Sans CJK TC, used only by the tests,
  [SIL Open Font License 1.1](third_party/noto-sans-cjk/OFL.txt).
