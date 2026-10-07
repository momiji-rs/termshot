# Using termshot

The full reference for the CLI: what it reads, what it draws, every option, and how it fails.
The [README](../README.md) has the short version; kitty graphics and Sixel are in [Images in
PTY logs](images.md).

- [Logs and the grid](#logs-and-the-grid)
- [asciinema recordings](#asciinema-recordings)
- [Text and JSON output](#text-and-json-output)
- [Options](#options)
- [Output and exit status](#output-and-exit-status)
- [What it draws](#what-it-draws)
- [Glyphs and fonts](#glyphs-and-fonts)
- [Font collections and variable fonts](#font-collections-and-variable-fonts)
- [Colours and padding](#colours-and-padding)

## Logs and the grid

```sh
./termshot examples/reply-sent.pty reply-sent.png
```

The binary carries JetBrains Mono, so it needs no other files. The grid defaults to 100×30 and
the font pixel height to 48. Give the size the capture used:

```sh
./termshot --size 120x40 session.pty session.png
./termshot --px 24 --font path/to/font.ttf session.pty session.png
cat session.pty | ./termshot - - > screen.png   # stdin to stdout
./termshot --fallback-font /path/to/cjk.ttf session.pty session.png
```

To screenshot a TUI that is still running, drive it in tmux and capture the pane. tmux ends
each row with a bare LF, so pass `--lf-newline` and the pane's size. The capture doesn't say
where the cursor is, so ask tmux and pass it with `--cursor`:

```sh
tmux new-session -d -s app -x 100 -y 30 top
cursor=$(tmux display -p -t app '#{?cursor_flag,#{cursor_x}#,#{cursor_y},none}')
tmux capture-pane -t app -e -p | ./termshot --lf-newline --size 100x30 --cursor "$cursor" - top.png
```

## asciinema recordings

An [asciinema](https://asciinema.org) recording works as the log too, in either format,
asciicast [v2](https://docs.asciinema.org/manual/asciicast/v2/) or
[v3](https://docs.asciinema.org/manual/asciicast/v3/). The grid takes the recording's size, so
`--size` isn't needed:

```sh
asciinema rec demo.cast
./termshot demo.cast demo.png
```

A log is read as a cast when its first line, from the first byte, is a JSON object with a
`"version"` member at its top level; the rest of the line need not be valid, so a damaged
header is refused rather than drawn as text. Terminal output rarely starts that way; when it
does, `--raw` reads the log as raw output, and a log taken for a cast that fails to read says
so. `--cast` reads the log as a cast whatever it starts with, which matters only for the error
you get. termshot replays the data of the output (`"o"`) events, in the order the file has
them, and ignores input (`"i"`), markers (`"m"`), exit (`"x"`) and other events. The size is
the header's (`width` and `height` in v2, `term.cols` and `term.rows` in v3), or that of the
last resize (`"r"`, `"COLSxROWS"`) event, as the final screen is the one drawn. The grid has
that one size from the start rather than changing mid-replay: full-screen programs redraw when
resized, so their last frame is right, but text printed before a resize may wrap where it did
not in the terminal. `--size`, or the original form's cols and rows, override it, each
dimension on its own. A recording larger than 500×200 needs `--size` (exit 2). Timing is not
used, so the replay doesn't depend on it.

The JSON is read strictly: UTF-8, with every escape including `\uXXXX` surrogate pairs, no
duplicate keys, nesting at most 16 deep, and every line after the header an event (in v3, or a
`#` comment), so a blank line is refused too. A lone surrogate, invalid UTF-8, a truncated
line, a wrong type, a size that is not a whole number, or a negative or infinite time is
refused with its line, and for bad JSON or UTF-8 its column (exit 1), as for an unusable font.
For a raw log, the size isn't guessed from where the cursor went; give it with `--size`.

## Text and JSON output

To check what a screen shows rather than how it looks (in a test, or as an agent), write it as
text. It is laid out as `tmux capture-pane -p` prints it: a line per row, trailing spaces
trimmed, each character followed by its combining marks. For logs without images, omitting the
PNG skips font loading, drawing and PNG encoding; how much time that saves depends on the log
(the benchmark's `text` suite times such runs, see [performance](performance.md)). A kitty
placement or a Sixel image still needs font metrics to replay cursor movement, so the font is
read for it even for text/JSON-only output; images themselves are not included in these
formats:

```sh
./termshot --text - session.pty | grep -q 'Saved'        # text only, to stdout
./termshot --text screen.txt session.pty screen.png     # both
```

`--json` adds the colours, the attributes and the cursor. Each row is a line of runs of cells
that look alike, with the column each starts at (a wide character takes two):

```json
{"cols":100,"rows":30,"cursor":{"col":2,"row":5,"shape":"block"},"lines":[
[{"col":0,"text":"ok","fg":"#00cd00","bg":"#111823","bold":true},{"col":2,"text":" done","fg":"#dbe7f7","bg":"#111823"}],
...
]}
```

`cursor` is null when the log hides it; its `shape` is `block`, `underline` or `bar`. `bold`,
`italic`, `underline`, `double_underline` and `strike` appear only when set. Blank cells that
end a row are left out unless their background or a line shows. A run's `text` has each
character followed by its combining marks, as in `--text`, so a mark takes no column of its
own. Colours are as drawn: under the palette (see [Colours and padding](#colours-and-padding)),
with reverse video and dim already applied, and concealed text has `fg` equal to `bg`.

## Options

| option | |
|---|---|
| `-f`, `--font FILE` | TrueType or OpenType (CFF or CFF2) font (default: built-in JetBrains Mono); `FILE#N` or `FILE#NAME` picks a face of a collection, `FILE#wght=700` an instance of a CFF2 variable font |
| `--fallback-font FILE` | TrueType or OpenType (CFF or CFF2) font for characters the first lacks, such as CJK; faces as for `--font` |
| `-p`, `--px N` | font pixel height, above 0 and below 256 (default 48) |
| `-s`, `--size CxR` | grid columns × rows, up to 500×200 (default: a cast's size, else 100x30) |
| `--cast` | read the log as an asciinema `.cast` (v2 or v3), even if its first line isn't a header |
| `--raw` | read the log as raw PTY output, even if its first line looks like a cast's header |
| `--lf-newline` | treat each bare LF as CR LF, for logs not captured through a PTY; a final bare LF ends the last line instead of scrolling |
| `--cursor COL,ROW` or `none` | draw the cursor there, counting from 0 as tmux's `#{cursor_x},#{cursor_y}` do, or not at all (default: where the log leaves it, unless it hides it) |
| `--cursor-shape block`, `underline` or `bar` | draw the cursor as that shape (default: the one the log sets with DECSCUSR, or a block) |
| `--text FILE` | write the screen as text, a line per row with trailing spaces trimmed; the PNG is then optional |
| `--json FILE` | write the screen as JSON: the cursor and its shape, and per row the runs of cells alike in colour and attributes; the PNG is then optional |
| `--palette FILE` | the default colours and the 16 named ones, in kitty's colour keys (see [Colours and padding](#colours-and-padding)) |
| `--fg #RRGGBB`, `--bg #RRGGBB` | the default foreground or background, over the palette's |
| `--padding N` or `X,Y` | a margin of N pixels around the cells, or X left and right and Y above and below, 0 to 1024, in the default background (default 0) |
| `-v`, `--verbose` | print the cell and image size, the face of each collection and the instance of each variable font to stderr |
| `-h`, `--help`, `-V`, `--version` | |

## Output and exit status

It prints nothing on success, except a hint on stderr when the log (for a cast, its output) has
line feeds but no CR, which means it was probably not captured through a PTY and needs
`--lf-newline`, and a warning when a font maps a character to an empty glyph, so it was drawn
as a box. The warning names the first such cell, the font, and what to pass instead. Exit
status is 0 when done; 1 when a file can't be read or written, a `.cast` is malformed, or the
font is unusable; and 2 for bad arguments, including a malformed `--palette` file, an image
over 2^27 pixels (its padding included) and a cast larger than 500×200 without `--size`.
termshot won't write a PNG to a terminal, and only one output can be `-`. A failed run removes
the output files it created.

The original form, `termshot <log> <out.png> <font.ttf> [px] [cols] [rows]`, still works.

## What it draws

It draws one final frame, not an animation. The screen model follows xterm and covers:

- autowrap, scrolling, scroll regions, inserting and deleting lines, and the alternate screen
  that full-screen programs use (`vi`, `less`)
- cursor movement, tabs, erase, inserting and deleting characters, saving the cursor, and DEC
  line drawing (`ESC ( 0`)
- the cursor, drawn where the log leaves it, unless the log hides it (`ESC [ ? 25 l`): a block
  in reverse video, or the underline or bar a program picks with DECSCUSR (`ESC [ 5 SP q`, as
  shells in vi mode and editors do for insert mode), drawn steady

`tests/vt/` checks this against tmux, on short cases and on recorded `ls`, `less` and `vi`
sessions. A bare LF moves down without returning to column 0, as in a terminal; logs captured
through a PTY already have CR LF. For output that has bare LFs (a text file, `cmd > out.log`),
pass `--lf-newline`.

Colors are the 16 and 256 color palettes (xterm's defaults) and 24-bit color, in the `;` and
`:` forms. Bold, dim, italic, underline, double underline, strike-through, reverse video and
hidden text are drawn; blink is not. Italic is the font's own glyph slanted by 12 degrees, and
box drawing stays upright in it. Wide characters (CJK, fullwidth forms, emoji) take two cells.
A combining mark composes with the character before it when Unicode has a precomposed form (e +
U+0301 is é); otherwise the cell keeps it, up to four marks, and draws it over the character,
so Thai, Hebrew points and stacked Latin accents show
([#14](https://github.com/momiji-rs/termshot/issues/14)). There is no shaping: a mark sits
where its font draws it, stacked marks may overlap, and Indic scripts are only approximate.
Emoji sequences (ZWJ, skin tones, VS16) are not joined into one picture.

## Glyphs and fonts

`M` is snapped to a whole number of pixels so box-drawing joints meet. All box drawing and
block elements (U+2500–U+259F: light, heavy, double and dashed lines, corners, tees, arcs,
diagonals, eighths, shades and quadrants) are painted as geometry inside their cell, so lines
join with any neighbour at any size; `tests/boxes.c` checks every one against its Unicode name.
Other characters come from the font, then from `--fallback-font`, which is sized to the same
height and centered in the cell; a character neither has is drawn as an outlined box, except
for spaces, the line and paragraph separators, and the blank Braille pattern U+2800. Wide
characters (CJK, fullwidth forms, emoji, by Unicode 17 widths) take two cells and are centered
over both (on a one-column screen, where no row can hold two, they take the one cell); a
combining mark merges into the character before it when Unicode has the precomposed form (e +
U+0301 is é); otherwise the cell keeps up to four marks, and each is drawn over the character
in its colours, from the font or else `--fallback-font` (a mark neither has is left out, not
boxed). Without shaping (no GPOS anchors), a mark its font draws left of its origin, as most
fonts do, is drawn from where the character ends; one drawn right of its origin, as in
right-to-left fonts, is centered over the character. Joiners, variation selectors, Hangul
fillers and the other default-ignorable characters are kept in `--text` and `--json` but draw
nothing. An SGR reset uses foreground `#dbe7f7` on background `#111823` unless a palette says
otherwise.

The font may have TrueType (`glyf`), CFF or CFF2 outlines, so `.ttf`, `.otf` and collections
such as Noto Sans CJK's `.ttc` all work. A variable font with CFF2 outlines (such as
`NotoSansCJKtc-VF.otf`) is drawn at its default instance, which for Noto Sans CJK is the Thin
weight, unless you choose another after a `#` (see [Font collections and variable
fonts](#font-collections-and-variable-fonts)). Color emoji fonts are bitmaps, not outlines, so
emoji need a monochrome outline font such as Noto Emoji. A glyph with no outline counts as
missing, so the emoji of a color font that has `glyf` (Apple Color Emoji) go on to
`--fallback-font` or are drawn as boxes rather than left blank, with a warning. stb_truetype
trusts the file it reads, so termshot first checks every structure stb will use
(`src/font.rs`), and runs CFF and CFF2 charstrings itself (`src/cff.rs`), with a limit on the
work a glyph may take; stb only rasterizes the outline. A damaged or hostile font is refused
with a reason, and the run exits 1.

## Font collections and variable fonts

A font collection (`.ttc`) holds several faces, often one per script or region, and termshot
draws with one. Pick it after a `#`: `FILE#3` by number, counting from 0 as `fc-list : file
index family` does, or `"FILE#Family Name"` by its full or family name, ignoring case. Without
a `#`, the first face is used and a hint on stderr lists the others; `FILE#0` uses the first
without the hint. A face that doesn't exist, or a name that two faces share, is refused with
the list (exit 1), and `-v` prints the face used. If the file's own name has a `#` in it, it is
read as that file.

A variable font with CFF2 outlines can be drawn at another instance: give its axis settings in
a last `#` part, `TAG=VALUE` separated by commas, as in `NotoSansCJKtc-VF.otf#wght=700` or
`FILE.ttc#1#wght=700,wdth=90`. Values are in the axis's own units (`hb-info --list-variations
FILE` lists them), clamped to its range, and the outlines match HarfBuzz's (`hb-view
--variations`). An axis left out stays at its default, and `-v` prints the instance. Bad syntax
exits 2. An axis the font doesn't have exits 1 and lists those it has; a TrueType or CFF font,
whose outlines don't vary here, exits 1 with that reason. The metrics vary with the instance
too, as in HarfBuzz: each glyph's advance by `HVAR`, so a heavy weight gets wider cells, and
the ascender, descender and line gap by `MVAR`.
## Colours and padding

termshot draws with its own default colours, foreground `#dbe7f7` on background `#111823`, and
xterm's 16 named colours. `--palette FILE` replaces any of them. It takes kitty's colour keys,
so these lines of a kitty theme work as they are:

```
# Solarized Dark
foreground #839496
background #002b36
color0  #073642
color1  #dc322f
...
color15 #fdf6e3
```

Each line is a key and a `#rrggbb` colour: `foreground`, `background`, or `color0` to `color15`
(SGR 30–37 and 40–47 are 0 to 7, 90–97 and 100–107 are 8 to 15, and so are `38;5;N` and
`48;5;N` for N below 16). Blank lines and lines starting with `#` are skipped. Any other line,
a key given twice, or a key termshot doesn't apply (`cursor`, `color16`, ...) is refused with
its line number (exit 2), so a typo isn't ignored, as is text that isn't UTF-8; so is a file
over 64 KiB, which no palette needs. A file that can't be read exits 1. To take a whole kitty
theme, keep those keys only:

```sh
grep -E '^(foreground|background|color([0-9]|1[0-5]))[[:space:]]' theme.conf > palette.conf
./termshot --palette palette.conf session.pty session.png
./termshot --bg '#000000' --fg '#ffffff' session.pty session.png   # just the defaults
```

`--fg` and `--bg` set the default colours on their own, over the file's. Colours 16 to 255 (the
6×6×6 cube and the greys) and 24-bit colours keep their values, as in terminals.

The palette applies as the log is replayed, as a terminal applies its own: a cell keeps the
colour its character was printed in, so `--json` reports the colours under the palette. What
depends on the default colours follows it. The block cursor is the cell in reverse video; the
underline and bar cursors are the default foreground, or the default background on a cell whose
background is the default foreground; dim and concealed text mix the palette's colours; and
kitty images below the cell backgrounds (`z` under −2^30) show only through cells whose
background is the palette's default, compared by value as kitty does. Bold doesn't brighten a
named colour, as before. kitty's Unicode placeholders name their image by colour numbers, not
values, so a palette never changes which image a cell shows.

`--padding N` draws a margin of N pixels around the cells, and `--padding X,Y` one of X pixels
left and right and Y above and below, each from 0 to 1024, in the default background. It is a
frame around the image termshot draws without it: the cells, glyphs, images and cursor move by
the margin and are cut at the cells' edges as before, and the cell size, so where an image
moves the cursor, doesn't change. The margin counts towards the 2^27-pixel limit, and `-v`
prints the padded size. `--text` and `--json` don't change.
