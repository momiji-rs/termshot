# Noto Sans Thai, Hebrew and Devanagari, subset

`NotoSans-Marks-Subset.ttf` is a test font, not a default: a few base letters and the
combining marks the pixel goldens draw over them (#14), from three Noto fonts merged into one
TrueType font of 5,332 bytes, small enough to keep in the repository. The goldens use it as
`--fallback-font` beside the built-in JetBrains Mono, which has none of these scripts.

The fonts are Noto Sans Thai 2.002, Noto Sans Hebrew 3.001 and Noto Sans Devanagari 2.007,
© 2022 and 2024 The Noto Project Authors, each licensed under the
[SIL Open Font License 1.1](OFL.txt) with no Reserved Font Name. They are the unhinted
`<Family>-Regular.ttf` of each, from `https://notofonts.github.io/<script>/fonts/<Family>/unhinted/ttf/`:

| file | sha256 |
|---|---|
| `NotoSansThai-Regular.ttf` | `ac0845b127e2e8b0f47a9193d89ed21fc952c0e669ad7ace839e4df9b5eb48bc` |
| `NotoSansHebrew-Regular.ttf` | `2f50c6bfe826a79562654d0e6a2140c4c8f41f612d66e2c80bf4090bf46920d0` |
| `NotoSansDevanagari-Regular.ttf` | `8400e48d4ecdd78ad7a5a10d892274f2e8faa250fd0d0762629986dbc98ef9b3` |

They were cut and merged with fontTools 4.66.1. termshot does no shaping, so the layout
tables go:

```sh
opts="--drop-tables+=GSUB,GPOS,GDEF,STAT,DSIG --no-hinting --name-IDs=* --name-languages=0x409 --notdef-outline"
pyftsubset NotoSansThai-Regular.ttf $opts \
    --unicodes=U+0E01,U+0E1B,U+0E31,U+0E34-0E3A,U+0E47-0E4E --output-file=thai.ttf
pyftsubset NotoSansHebrew-Regular.ttf $opts \
    --unicodes=U+05B4,U+05B7-05B9,U+05BC,U+05C1-05C2,U+05D1,U+05D5,U+05DC,U+05DD,U+05E9 --output-file=hebrew.ttf
pyftsubset NotoSansDevanagari-Regular.ttf $opts \
    --unicodes=U+0902,U+0915,U+0924,U+0926,U+0928,U+092E,U+0938,U+0939,U+093F-0941,U+0947,U+094D \
    --output-file=devanagari.ttf
pyftmerge thai.ttf hebrew.ttf devanagari.ttf --output-file=NotoSans-Marks-Subset.ttf
```

It maps those 43 characters, 47 glyphs in all. Its name table is Noto Sans Thai's, which pyftmerge keeps.
