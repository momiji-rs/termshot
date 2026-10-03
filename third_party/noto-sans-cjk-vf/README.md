# Noto Sans CJK TC variable, subset

`NotoSansCJKtc-VF-Subset.otf` is a test font, not a default: a variable font with CFF2
outlines (one axis, `wght` 100–900), small enough to keep in the repository. termshot draws
its default instance, which is the Thin weight (100). It is Noto Sans CJK TC, version 2.004,
© 2014-2021 Adobe (http://www.adobe.com/), licensed under the [SIL Open Font License 1.1](OFL.txt)
with no Reserved Font Name.

It was cut from `Sans/Variable/OTF/NotoSansCJKtc-VF.otf` in
[notofonts/noto-cjk](https://github.com/notofonts/noto-cjk) at commit 165c01b
(sha256 `fb6fd590a4bcfe1b5d71b8bb0054719256b815e8fc700dd70fd171a3660a9e6b`), with
harfbuzz-utils 14.4.0, as `third_party/noto-sans-cjk/` was:

```sh
hb-subset NotoSansCJKtc-VF.otf \
    --text="骨直角永東京台灣測試字型漢字中文繁體簡體龍鬱鑿齉" --unicodes+=20-7e \
    -o NotoSansCJKtc-VF-Subset.otf
```

It maps printable ASCII and those 22 characters, 176 glyphs in all, in 5 font dicts chosen by
a format 3 FDSelect. The variation store has one region, and 173 of the glyphs
use `blend` (4,745 times in all). Neither the font nor hb-subset's output uses subroutines; the hand-made
fonts in `src/cff_tests.rs` cover those. `tools/cff2-outlines.sh` records how HarfBuzz draws
each character, which the tests compare with `src/cff.rs`.
