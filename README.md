# termshot

Replay a captured PTY log into a PNG. No window, no crates.io dependencies.
The same source builds on macOS and Linux and writes the same pixels.

termshot reads bytes a terminal already emitted, rebuilds the cell grid, and paints it.

![A 100 by 30 demo inbox, rasterized from examples/reply-sent.pty](docs/reply-sent.png)

## Build

A C compiler and rustc are enough.

```sh
./build.sh
```

The binary links libc and libm.

## Run

```sh
./termshot examples/reply-sent.pty reply-sent.png \
  third_party/jetbrains-mono/JetBrainsMono-Regular.ttf 48
```

`48` is the font pixel height. Columns and rows default to 100 and 30, which is the size of the sample logs. Pass them after the pixel height when the capture used another size:

```sh
./termshot session.pty session.png path/to/font.ttf 48 120 40
```

`M` is snapped to a whole number of pixels so box-drawing joints meet. `─ │ ┌ ┐ └ ┘ ╭ ╮ ╯ ╰ ▀ █` are painted as geometry. Other characters come from the font. An SGR reset uses foreground `#dbe7f7` on background `#111823`.

## Samples

`examples/reply-sent.pty` and `examples/draft-ready.pty` are captures from the [crisp-tui](https://github.com/solcreek/crisp-tui) demo inbox. The customers and messages are fake. At pixel height 48 the image is 2200×1440.

## Vendored files

The program is MIT. Two things next to it keep their own terms:

- `third_party/stb/` is [stb](https://github.com/nothings/stb) `stb_truetype.h` 1.26 and `stb_image_write.h` 1.16, public domain.
- `third_party/jetbrains-mono/` is JetBrains Mono Regular, [SIL Open Font License 1.1](third_party/jetbrains-mono/OFL.txt).
