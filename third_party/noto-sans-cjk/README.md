# Noto Sans CJK TC, subset

`NotoSansCJKtc-Subset.otf` is a test font, not a default: a CID-keyed CFF font small enough
to keep in the repository. It is Noto Sans CJK TC Regular, version 2.004, © 2014-2021 Adobe
(http://www.adobe.com/), licensed under the [SIL Open Font License 1.1](OFL.txt) with no
Reserved Font Name. It was cut from face 3 of `NotoSansCJK-Regular.ttc` in Arch's
noto-fonts-cjk 20240730-1, with harfbuzz-utils 14.4.0:

```sh
hb-subset /usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc --face-index=3 \
    --text="骨直角永東京台灣測試字型漢字中文繁體簡體龍鬱鑿齉" --unicodes+=20-7e \
    -o NotoSansCJKtc-Subset.otf
```

It maps printable ASCII and those 22 characters, 176 glyphs in all. `--unicodes+=` adds to
`--text`; `--unicodes=` would replace it.
