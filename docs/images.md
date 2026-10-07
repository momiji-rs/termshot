# Images in PTY logs

What termshot draws of kitty graphics and Sixel, and the limits it keeps. The
[README](../README.md) has the short version; [kitty-graphics.md](kitty-graphics.md) has the
regression evidence.

## kitty graphics

Kitty graphics sent inline (`t=d`) render above the text: RGB (`f=24`), RGBA (`f=32`, the
default), and PNG (`f=100`), including `m=1`/`m=0` chunks and zlib-compressed payloads (`o=z`;
a compressed PNG gives its size in `S`). `a=T` transmits and places an image; `a=t` only stores
it, under an id `i` or a number `I`, and each `a=p` places a stored one again, sharing its
pixels. Images start at the cursor, or `X`/`Y` pixels into its cell (at most a pixel short of
the cell's edge), use their native pixel size or fit a `c`/`r` cell rectangle while preserving
aspect ratio, and blend alpha over the existing screen. `x`, `y`, `w`, `h` choose a source
rectangle in pixels, and the part of it inside the image is shown; that crop's aspect ratio is
the one kept, and an empty crop draws nothing. Scaling uses deterministic nearest-neighbor
sampling. Cell dimensions come from the selected font and `--px`. `C=1` keeps the cursor in
place; otherwise it moves as kitty moves it: right by the placement's columns and down by its
rows less one, to the next row's start if that reaches the right edge, scrolling the region up
if it passes the bottom margin.

A placement id `p` names one placement of an image: putting the same `i,p` again moves it.
Deletes follow kitty's selectors: all (`d=a`, the default), by id (`i`, with `p`), by number
(`n`), by id range (`r`), at the cursor (`c`), at a cell (`p`, `q` with a z-index), in a column
(`x`), a row (`y`), or by z-index (`z`). Lowercase keeps the image data for another `a=p`;
uppercase also frees the images it leaves without a placement. Retransmitting an id replaces
its image and removes its placements. Images draw by `z`, then the order images and placements
were made: from 0 over the text, below 0 under the text but over every cell background, and
below -1,073,741,824 under the backgrounds that are not the default colour, so they show only
through default ones. Reverse-video cells and the block cursor are opaque there, as in kitty.
The underline and bar cursors are drawn with the text, as kitty draws them: over the images
under it, under those of `z` 0 and up. Only images wholly inside a scrolling region move and
clip at its edges; images crossing a margin stay stationary. Full-screen erase and reset remove
every placement but the virtual ones and free every stored image left without one, as kitty
does. A relative placement (`P` and `Q` name a parent image and placement) starts `H`, `V`
cells from its parent's top left cell, follows the parent when it moves or scrolls, and goes
when the parent goes; it never moves the cursor. A missing parent, a cycle, or a chain of more
than 8 links refuses the put. Explicit image ids and numbers must be nonzero; omitting `i` is
valid. Main and alternate screens keep separate images. See [the regression
evidence](kitty-graphics.md).

### Unicode placeholders

Unicode placeholders work as in kitty (`kitten icat --unicode-placeholder`, tmux, editors):
`U=1` makes a virtual placement of `c` x `r` cells, which draws nothing itself, and each
U+10EEEE cell shows the part of the image under it. The cell's foreground colour is the image
id (`38;5;n` is `n`, `38;2;r;g;b` is 0xRRGGBB, a third diacritic the high byte), its underline
colour (`58`) the placement id, and its first two diacritics, from kitty's
`rowcolumn-diacritics.txt`, the row and column; missing ones are inherited from the cell to the
left. The image is fitted into the placement's cells keeping its aspect ratio, centered, at
z-index -1, over the cell's background. The placeholders are ordinary text, so the image moves,
scrolls and is erased with them. A placeholder whose image or placement does not exist draws
nothing. `d=i`, `n` and `r` delete virtual placements; the other selectors, full-screen erase
and reset leave them. A relative placement may name a virtual parent: it starts from the top
row and the leftmost column the image shows in. Placeholder cells are drawn blank; `--text` and
`--json` keep U+10EEEE and its diacritics.

### Animation

Animated images (`kitten icat` with a GIF, for one) are drawn as a still, since a log has no
timeline: each image shows the frame an explicit `a=a` with `c` last made current, or else its
first frame. Frames (`a=f`) and compositions (`a=c`) are built as kitty builds them; gaps, the
animation state and loop counts change nothing. See [Animation](kitty-graphics.md#animation).

This is a subset, not full kitty emulation: file/shared-memory transfer is not supported.
Unsupported or malformed commands are ignored without printing their payload. PNG images may be
compressed internally as usual. The log must contain the original escape sequences and image
bytes; a plain `tmux capture-pane` text capture cannot recover them. This does not make every
image-using TUI capture compatible automatically. Advanced kitty work was tracked in
[#44](https://github.com/momiji-rs/termshot/issues/44).

## Sixel

Sixel images (`ESC P P1;P2;P3 q … ESC \`) are drawn too, following xterm's decoder (its
`graphics_sixel.c`, patch 412) and the VT340 it emulates:

- Colour introducers `#Pc` select a register and `#Pc;Pu;Px;Py;Pz` define it, in HLS (`Pu` 1,
  DEC hues: 0 is blue, 120 red, 240 green) or RGB (`Pu` 2), in percent, rounded half up to 8
  bits. Each image has its own 1,024 registers, starting from the VT340's 16 colours (the rest
  black), as xterm's default private colour registers; register numbers wrap at 1,024. Drawing
  starts in register 3, as in xterm. Pixels keep their register, so redefining one recolours
  what it drew: the palette at the end of the image applies. A definition out of range or with
  the wrong number of parameters is ignored.
- Repeats (`!Pn`), graphics carriage return (`$`) and next line (`-`), and raster attributes
  (`"Pan;Pad;Ph;Pv`). Pixels are square device pixels at their native size, as in xterm, which
  also ignores `P1` and `Pan;Pad`; cell dimensions come from the font and `--px`, as for
  kitty's native sizing. The image is as large as `Ph`x`Pv` or its pixels reach, whichever is
  larger; an extent given as 0 or left empty counts as 1, as in xterm.
- `P2` 1 leaves pixels no sixel set transparent. `P2` 0 or 2 paints the area the raster
  attributes declared before the first sixel with register 0, as xterm does; pixels past it
  stay transparent.
- With Sixel scrolling on (the default), the image starts at the cursor, and the cursor ends on
  the last text row the image covers, in the same column, so the program's next newline goes
  below it. An image that would pass the bottom margin first scrolls the region up; its top is
  cut off if it is taller than the region. DECSDM (`CSI ? 80 h`) instead draws at the top left
  corner, clipped at the bottom, without scrolling or moving the cursor.
- The image joins the kitty image store as an unnamed image drawn above the text, so it shares
  the layering, scrolling, erase and storage limits above. Unlike a kitty image, its pixels
  belong to the cells, as in xterm: a character written there later clears them in the cells it
  takes, and erasing below or above the cursor (ED 0 or 1) clears them in the rows below or
  above the cursor's, though not in its own row. Only `ESC \` commits an image; BEL, CAN, SUB,
  another escape or the end of the log discard it, and C1 controls (such as the 8-bit ST) are
  not recognised. A DCS that is not Sixel (`DECRQSS`, `XTGETTCAP`, ...) is skipped. An image
  wider or taller than 8,192 pixels or over 4,194,304 pixels is refused whole, before its
  pixels are allocated, and a repeat costs nothing beyond those bounds. Since a few bytes can
  declare a large image or draw over the same pixels again and again, a log may write at most
  16,777,216 Sixel pixels plus 256 for each byte of Sixel data, counting every pixel a sixel
  sets, each time it sets it, and every pixel of each finished image; an image past that budget
  is refused.

EL, ECH, ICH and DCH leave the pixels, as in xterm; IL and DL move images as they move kitty's,
where xterm leaves them. An image with no pixels moves nothing. Non-square pixels from `P1` or
`Pan;Pad` (xterm ignores them too), DECSET 8452 (the cursor to the right of the image), shared
colour registers (`CSI ? 1070 l`) and ReGIS are not supported.

## Limits

Limits per screen are 1,024 placements, and 4,096 stored images holding 16 MiB of RGBA pixels.
Past the image limits, an upload first frees every image without a placement, then the least
recently placed ones, as kitty's storage quota does; a placement past its limit is discarded.
Each upload is limited to 16 MiB of decoded payload and 8,192 pixels per source axis (at most
4,194,304 source pixels). A display rectangle is limited to 16,777,216 pixels per axis. The PNG
decoder has a separate 64 MiB allocation budget, including inflation. As in kitty, an RGB or
RGBA payload may be at most 10 bytes over its decoded size, which are ignored, or 1,024 bytes
if compressed (`o=z`); a PNG payload at most 16 MiB. Over-limit commands are discarded.
