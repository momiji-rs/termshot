#!/bin/sh
# Build, then run the parser unit tests, the CLI exit-code checks, the
# pixel goldens and the library test. Renders land in target/test/ and are
# kept for inspection.
#
#   ./test.sh                    run everything
#   ./test.sh --update-goldens   rewrite tests/goldens.txt from this build
#   SANITIZE=1 ./test.sh         build the C (stb_glue.c) with ASan + UBSan (macOS clang)
set -eu
cd "$(dirname "$0")"
mode=check
[ "${1:-}" = --update-goldens ] && mode=update
out=target/test
mkdir -p "$out"
font=third_party/jetbrains-mono/JetBrainsMono-Regular.ttf

if [ "${SANITIZE:-}" = 1 ]; then
    [ "$(uname)" = Darwin ] || { echo "SANITIZE=1 is wired up for macOS clang only" >&2; exit 2; }
    # rustc links with -nodefaultlibs, so the sanitizer runtime is named explicitly.
    rt="$(cc -print-resource-dir)/lib/darwin"
    export CFLAGS="-O1 -g -fno-omit-frame-pointer -fsanitize=address,undefined -fno-sanitize-recover=undefined"
    export RUSTC_LINK_ARGS="-C link-arg=-fsanitize=address,undefined -C link-arg=$rt/libclang_rt.asan_osx_dynamic.dylib -C link-arg=-Wl,-rpath,$rt"
    # termshot handles a failed allocation (exit 2), so ASan returns null for
    # one it can't make, as malloc would, instead of aborting; a unit test
    # asks for a raster no memory can hold.
    export ASAN_OPTIONS=detect_leaks=0:abort_on_error=1:allocator_may_return_null=1
fi

./build.sh

echo "== unit tests"
# The library's (src/lib.rs, linking the C as build.sh does), then the CLI's
# (src/main.rs, linking the library).
# shellcheck disable=SC2086
rustc --edition 2021 --test src/lib.rs -o "$out/unit" \
    -L native="$PWD" -l static=termshot_c ${RUSTC_LINK_ARGS:-}
"$out/unit" -q
# shellcheck disable=SC2086
rustc --edition 2021 --test src/main.rs -o "$out/cli-unit" --extern termshot=libtermshot.rlib ${RUSTC_LINK_ARGS:-}
"$out/cli-unit" -q

echo "== deflate matches stb"
# The C harnesses link the Rust they call (src/deflate.rs, src/geometry.rs
# and, built with --cfg termshot_render, the render) as a static library,
# with the system libraries rustc names for it (tests/rust_lib.sh).
rust="$out/libtermshot_rust.a"
rust_libs=$(tests/rust_lib.sh "$rust")
# shellcheck disable=SC2086
cc tests/deflate_diff.c "$rust" -o "$out/deflate_diff" -O2 -Wno-deprecated-declarations \
    -I third_party/stb ${CFLAGS:-} $rust_libs
"$out/deflate_diff"
# The safe 16-lane Adler-32, which x86-64 builds would otherwise not use.
if [ "$(uname -m)" = x86_64 ]; then
    portable_libs=$(tests/rust_lib.sh "$out/libtermshot_rust_portable.a" --cfg termshot_portable_adler)
    # shellcheck disable=SC2086
    cc tests/deflate_diff.c "$out/libtermshot_rust_portable.a" -o "$out/deflate_diff_portable" -O2 \
        -Wno-deprecated-declarations -I third_party/stb ${CFLAGS:-} $portable_libs
    "$out/deflate_diff_portable"
fi

echo "== box drawing and blocks"
# shellcheck disable=SC2086
cc tests/boxes.c "$rust" -o "$out/boxes" -O2 -ffp-contract=off -Wno-deprecated-declarations \
    -I src -I third_party/stb -lm ${CFLAGS:-} $rust_libs
"$out/boxes"

echo "== glyph placement"
# The decoder is test-only, so it is built without sanitizers or our warnings.
cc -c tests/png_read.c -o "$out/png_read.o" -O2 -I third_party/stb
rm -f "$out/libpng_read.a"
ar rcs "$out/libpng_read.a" "$out/png_read.o"
# The render, which calls the stb glue tests/glyphs.c includes.
render="$out/libtermshot_render.a"
render_libs=$(tests/rust_lib.sh "$render" --cfg termshot_render)
# shellcheck disable=SC2086
cc tests/glyphs.c "$out/png_read.o" "$render" -o "$out/glyphs" -O2 -ffp-contract=off -Wno-deprecated-declarations \
    -I src -I third_party/stb -lm ${CFLAGS:-} $render_libs
"$out/glyphs" "$font" "$out/glyphs.png" "$out/hollow-A.ttf" "$out/fb-reference.png"

echo "== cli"
fail=0
expect() {
    want=$1
    shift
    set +e
    ./termshot "$@" >/dev/null 2>&1
    got=$?
    set -e
    if [ "$got" -ne "$want" ]; then
        echo "FAIL exit $got, want $want: termshot $*"
        fail=1
    fi
}
check() {
    if ! eval "$2"; then
        echo "FAIL $1"
        fail=1
    fi
}
log=examples/reply-sent.pty
# Exit status: 0 done, 1 unreadable or unwritable file or bad font, 2 bad arguments.
expect 2
expect 0 --help
expect 0 --version
expect 1 "$out/missing.pty" "$out/x.png" "$font"
expect 1 "$log" "$out/x.png" "$out/missing.ttf"
expect 1 "$log" "$out/x.png" --font README.md
expect 1 "$log" "$out/x.png" --fallback-font "$out/missing.ttf"
expect 1 "$log" "$out/x.png" --fallback-font README.md
expect 2 "$log" "$out/x.png" --fallback-font
expect 1 "$log" "$out/no-such-dir/x.png"
expect 2 "$log" "$out/x.png" "$font" 0
expect 2 "$log" "$out/x.png" "$font" 48 0
expect 2 "$log" "$out/x.png" "$font" 255 500 73
expect 2 "$log"
expect 2 "$log" "$out/x.png" --bogus
expect 2 "$log" "$out/x.png" --px
expect 2 "$log" "$out/x.png" --px 300
expect 2 "$log" "$out/x.png" --size 100
expect 2 "$log" "$out/x.png" --size 0x30
expect 2 "$log" "$out/x.png" -é
expect 2 "$log" "$out/x.png" --font "$font" "$font"
expect 2 "$log" "$out/x.png" "$font" 48 100 30 extra
# The built-in font, the original form, and options all render the same image.
./termshot "$log" "$out/builtin.png"
./termshot "$log" "$out/legacy.png" "$font" 48 100 30
./termshot --font="$font" -p48 --size 100x30 "$log" "$out/options.png"
check "builtin font matches the named font" 'cmp -s "$out/builtin.png" "$out/legacy.png"'
check "options match the original form" 'cmp -s "$out/options.png" "$out/legacy.png"'
# A fallback font only adds the characters the first lacks.
./termshot --fallback-font "$font" "$log" "$out/fallback.png"
check "an unused fallback font changes nothing" 'cmp -s "$out/fallback.png" "$out/legacy.png"'
# The glyph harness wrote a copy of the font with no outline for 'A', and the
# PNG draw_png makes of A with it and the font as the fallback. Without a
# fallback that A is a box; with one the CLI must make that same PNG. B is
# the same either way. tests/glyphs.c checks where the fallback's glyphs land.
printf 'B' | ./termshot --size 1x1 --cursor none - "$out/fb-plain.png"
printf 'B' | ./termshot --size 1x1 --cursor none --font "$out/hollow-A.ttf" - "$out/fb-b.png"
printf 'A' | ./termshot --size 1x1 --cursor none --font "$out/hollow-A.ttf" - "$out/fb-tofu.png" 2>/dev/null
printf 'A' | ./termshot --size 1x1 --cursor none --font "$out/hollow-A.ttf" --fallback-font "$font" - "$out/fb-drawn.png"
check "a font missing only A draws B as the font does" 'cmp -s "$out/fb-b.png" "$out/fb-plain.png"'
check "--fallback-font draws a glyph the font lacks" '! cmp -s "$out/fb-drawn.png" "$out/fb-tofu.png"'
check "--fallback-font draws it as draw_png does" 'cmp -s "$out/fb-drawn.png" "$out/fb-reference.png"'
# A face in a collection: the unit tests write $ttc, JetBrains Mono as "Face A"
# and a taller copy as "Face B".
ttc=$out/collection.ttc
expect 1 "$log" "$out/x.png" --font "$ttc#2"
expect 1 "$log" "$out/x.png" --font "$ttc#Face C"
expect 1 "$log" "$out/x.png" --font "$font#1"
expect 2 --font "$ttc#1" "$log" "$ttc"
cp "$ttc" "$ttc#x"
expect 2 "$log" "$out/x.png" --font "$ttc#x#1"
rm "$ttc#x"
check "a collection without a face says which it used, and lists the others" \
    './termshot --font "$ttc" "$log" "$out/ttc.png" 2>"$out/ttc.err" &&
     grep -q "the first, Face A, was used" "$out/ttc.err" && grep -qx "  #1  Face B" "$out/ttc.err"'
check "#0 picks the first face quietly" '[ -z "$(./termshot --font "$ttc#0" "$log" "$out/ttc-0.png" 2>&1)" ]'
check "the first face draws as the font it copies" 'cmp -s "$out/ttc-0.png" "$out/legacy.png"'
check "-v names the face" \
    './termshot -v --font="$ttc#face b" "$log" "$out/ttc-1.png" 2>&1 | grep -qx -- "--font face #1 Face B"'
check "another face draws differently" '! cmp -s "$out/ttc-1.png" "$out/legacy.png"'
# CFF outlines, as the font or the fallback. The unit tests write a CFF font
# whose CharStrings run past its table.
cjk=third_party/noto-sans-cjk/NotoSansCJKtc-Subset.otf
printf '東京' | ./termshot --size 4x1 --cursor none - "$out/cli-cff-tofu.png"
printf '東京' | ./termshot --size 4x1 --cursor none --fallback-font "$cjk" - "$out/cli-cff-fallback.png"
printf '東京' | ./termshot --size 4x1 --cursor none --font "$cjk" - "$out/cli-cff-font.png"
check "a CFF fallback font draws CJK" '! cmp -s "$out/cli-cff-fallback.png" "$out/cli-cff-tofu.png"'
check "a CFF font draws CJK" '! cmp -s "$out/cli-cff-font.png" "$out/cli-cff-tofu.png"'
expect 1 "$log" "$out/x.png" --font "$out/cff-past-table.otf"
check "a damaged CFF table is refused with a reason" \
    './termshot --font "$out/cff-past-table.otf" "$log" "$out/x.png" 2>&1 | grep -q "CFF table: INDEX at .* runs past the table"'
# CFF2 (a variable font, drawn at its default instance), the same ways.
vf=third_party/noto-sans-cjk-vf/NotoSansCJKtc-VF-Subset.otf
printf '東京' | ./termshot --size 4x1 --cursor none --fallback-font "$vf" - "$out/cli-cff2-fallback.png"
printf '東京' | ./termshot --size 4x1 --cursor none --font "$vf" - "$out/cli-cff2-font.png"
check "a CFF2 fallback font draws CJK" '! cmp -s "$out/cli-cff2-fallback.png" "$out/cli-cff-tofu.png"'
check "a CFF2 font draws CJK" '! cmp -s "$out/cli-cff2-font.png" "$out/cli-cff-tofu.png"'
check "a CFF2 font draws its default instance, not the CFF font" '! cmp -s "$out/cli-cff2-font.png" "$out/cli-cff-font.png"'
check "a CFF2 font is quiet on success" '[ -z "$(./termshot --font "$vf" "$log" "$out/cli-cff2-log.png" 2>&1)" ]'
expect 1 "$log" "$out/x.png" --font "$out/cff2-past-table.otf"
check "a damaged CFF2 table is refused with a reason" \
    './termshot --font "$out/cff2-past-table.otf" "$log" "$out/x.png" 2>&1 | grep -q "CFF2 table: INDEX at .* runs past the table"'
# Another instance of it, after a #: its weight axis runs from 100, the default, to 900.
printf '東京' | ./termshot --size 4x1 --cursor none --font "$vf#wght=900" - "$out/cli-cff2-900.png"
printf '東京' | ./termshot --size 4x1 --cursor none --font "$vf#wght=100" - "$out/cli-cff2-100.png"
printf '東京' | ./termshot --size 4x1 --cursor none --fallback-font "$vf#wght=900" - "$out/cli-cff2-fallback-900.png"
check "a CFF2 instance draws differently" '! cmp -s "$out/cli-cff2-900.png" "$out/cli-cff2-font.png"'
check "the default's own setting draws the default" 'cmp -s "$out/cli-cff2-100.png" "$out/cli-cff2-font.png"'
check "a CFF2 fallback font takes an instance" '! cmp -s "$out/cli-cff2-fallback-900.png" "$out/cli-cff2-fallback.png"'
check "a CFF2 instance sizes its cells by its HVAR advances (26x49 by default)" \
    './termshot -v --font "$vf#wght=900" "$log" "$out/cli-cff2-v.png" 2>&1 | grep -q "^advance 877 units .* cell 29x48 "'
check "-v names the instance, clamped" \
    './termshot -v --font "$vf#wght=1000" "$log" "$out/cli-cff2-v.png" 2>&1 | grep -qx -- "--font instance wght=900 (1000 clamped)"'
expect 2 "$log" "$out/x.png" --font "$vf#wght=bold"
expect 1 "$log" "$out/vary-fail.png" --font "$vf#wdth=50"
check "an unknown axis is refused with the axes, and leaves no output" \
    './termshot --font "$vf#wdth=50" "$log" "$out/vary-fail.png" 2>&1 | grep -q "no axis .wdth.; its axis is wght 100 to 900, default 100" &&
     [ ! -e "$out/vary-fail.png" ]'
expect 1 "$log" "$out/x.png" --font "$font#wght=700"
expect 1 "$log" "$out/x.png" --font "$cjk#wght=700"
# stdin and stdout.
./termshot - - < "$log" > "$out/piped.png"
check "stdin to stdout matches" 'cmp -s "$out/piped.png" "$out/legacy.png"'
# --lf-newline: a bare LF renders as CR LF, and a PTY log (all CR LF) is unchanged.
printf '\033[1mab\033[m\ncd\nef' | ./termshot --lf-newline --size 10x4 - "$out/lf-newline.png"
printf '\033[1mab\033[m\r\ncd\r\nef' | ./termshot --size 10x4 - "$out/crlf.png"
check "--lf-newline renders LF as CR LF" 'cmp -s "$out/lf-newline.png" "$out/crlf.png"'
./termshot --lf-newline "$log" "$out/lf-newline-pty.png"
check "--lf-newline leaves a PTY log unchanged" 'cmp -s "$out/lf-newline-pty.png" "$out/legacy.png"'
expect 2 "$log" "$out/x.png" --lf-newline=yes
# Under --lf-newline the final LF ends the last line: rows as tall as the grid keep the top row.
printf 'a\nb\nc\nd\n' | ./termshot --lf-newline --size 1x4 - "$out/lf-tall.png"
printf 'a\r\nb\r\nc\r\nd' | ./termshot --size 1x4 - "$out/crlf-tall.png"
check "--lf-newline keeps the top row of a full-height capture" 'cmp -s "$out/lf-tall.png" "$out/crlf-tall.png"'
# A final CR LF is not bare: a full-height PTY log scrolls with or without the flag.
printf 'a\r\nb\r\nc\r\nd\r\n' | ./termshot --lf-newline --size 1x4 - "$out/lf-crlf-end.png"
printf 'a\r\nb\r\nc\r\nd\r\n' | ./termshot --size 1x4 - "$out/crlf-end.png"
check "--lf-newline leaves a final CR LF alone" 'cmp -s "$out/lf-crlf-end.png" "$out/crlf-end.png"'
# The pipeline it is for: tmux capture-pane of a pane the size of the grid.
# Needs tmux, which CI doesn't have.
if command -v tmux >/dev/null; then
    socket="termshot-test-$$"
    # It ends with the cursor away from the text, which capture-pane loses; --cursor gives it back.
    printf '\033[31mred\033[m\r\nplain\r\n\r\nlast\033[2;3H' > "$out/pane.pty"
    tmux -L "$socket" -f /dev/null new-session -d -x 10 -y 4 "cat '$out/pane.pty'; sleep 30"
    tries=0
    until tmux -L "$socket" capture-pane -p | grep -q last || [ "$tries" -ge 50 ]; do
        sleep 0.1
        tries=$((tries + 1))
    done
    at=$(tmux -L "$socket" display -p '#{?cursor_flag,#{cursor_x}#,#{cursor_y},none}')
    tmux -L "$socket" capture-pane -e -p | ./termshot --lf-newline --size 10x4 --cursor "$at" - "$out/pane.png"
    tmux -L "$socket" kill-server
    ./termshot --size 10x4 "$out/pane.pty" "$out/pane-direct.png"
    check "tmux capture-pane renders like the bytes the pane was sent" 'cmp -s "$out/pane.png" "$out/pane-direct.png"'
else
    echo "skip: tmux capture-pane check (no tmux)"
fi
# --cursor: COL,ROW from 0 (COL may be the column count, as tmux reports a
# pending wrap) or none, in place of where the log leaves the cursor.
expect 2 "$log" "$out/x.png" --cursor
expect 2 "$log" "$out/x.png" --cursor 3
expect 2 "$log" "$out/x.png" --cursor -1,0
expect 2 "$log" "$out/x.png" --cursor 101,0
expect 2 "$log" "$out/x.png" --cursor 0,30
expect 2 "$log" "$out/x.png" --cursor hidden
check "--cursor names the grid it is off" './termshot --size 10x4 --cursor 0,4 "$log" "$out/x.png" 2>&1 | grep -q "off the 10x4 grid"'
printf 'ab' | ./termshot --size 10x4 --cursor none - "$out/cursor-none.png"
printf 'ab\033[?25l' | ./termshot --size 10x4 - "$out/cursor-hidden.png"
check "--cursor none hides the cursor" 'cmp -s "$out/cursor-none.png" "$out/cursor-hidden.png"'
printf 'ab\033[?25l' | ./termshot --size 10x4 --cursor 1,2 - "$out/cursor-at.png"
printf 'ab\033[3;2H' | ./termshot --size 10x4 - "$out/cursor-moved.png"
check "--cursor draws it there, even if the log hid it" 'cmp -s "$out/cursor-at.png" "$out/cursor-moved.png"'
printf 'ab' | ./termshot --size 10x4 --cursor=10,3 - "$out/cursor-pending.png"
printf 'ab\033[4;10H' | ./termshot --size 10x4 - "$out/cursor-last.png"
check "--cursor at the column count is the last column" 'cmp -s "$out/cursor-pending.png" "$out/cursor-last.png"'
# --cursor-shape: block, underline or bar, in place of the one the log sets.
expect 2 "$log" "$out/x.png" --cursor-shape
expect 2 "$log" "$out/x.png" --cursor-shape beam
check "--cursor-shape names the shapes" './termshot --cursor-shape Bar "$log" "$out/x.png" 2>&1 | grep -q "block, underline or bar"'
printf '\033[6 qab' | ./termshot --size 10x4 - "$out/shape-bar.png"
printf 'ab' | ./termshot --size 10x4 --cursor-shape bar - "$out/shape-bar-option.png"
check "--cursor-shape draws the shape DECSCUSR would" 'cmp -s "$out/shape-bar.png" "$out/shape-bar-option.png"'
printf '\033[6 qab' | ./termshot --size 10x4 --cursor-shape block - "$out/shape-block-option.png"
printf 'ab' | ./termshot --size 10x4 - "$out/shape-block.png"
check "--cursor-shape block undoes the log's shape" 'cmp -s "$out/shape-block.png" "$out/shape-block-option.png"'
check "a bar is not a block" '! cmp -s "$out/shape-bar.png" "$out/shape-block.png"'
printf '\033[6 qab\033[?25l' | ./termshot --size 10x4 - "$out/shape-hidden.png"
check "a hidden bar draws nothing" 'cmp -s "$out/shape-hidden.png" "$out/cursor-hidden.png"'
# --text: the screen as text, with the PNG or without it.
printf 'ab\r\ncd  \r\n\344\270\255x' | ./termshot --size 10x4 --text "$out/text.txt" -
printf 'ab\ncd\n\344\270\255x\n\n' > "$out/text-want.txt"
check "--text alone writes the rows, trimmed" 'cmp -s "$out/text.txt" "$out/text-want.txt"'
printf 'ab\r\ncd  \r\n\344\270\255x' | ./termshot --size 10x4 --text - - > "$out/text-stdout.txt"
check "--text - writes stdout" 'cmp -s "$out/text-stdout.txt" "$out/text-want.txt"'
./termshot --text "$out/text-png.txt" "$log" "$out/text-png.png"
check "--text leaves the PNG as it was" 'cmp -s "$out/text-png.png" "$out/builtin.png"'
check "--text needs no font without a PNG" './termshot --font README.md --text "$out/q.txt" "$log"'
printf '\033_Ga=p,i=9\033\\ab' > "$out/put-unsent.pty"
check "--text needs no font for a put of an image never sent" './termshot --font README.md --text "$out/q.txt" "$out/put-unsent.pty"'
printf '\033_Ga=t,i=9,f=24,s=1,v=1;/wAA\033\\\033_Ga=p,i=9\033\\' > "$out/put-sent.pty"
expect 1 --font README.md --text "$out/q.txt" "$out/put-sent.pty"
expect 2 "$log" --text
expect 2 --text - "$log" -
expect 1 --text "$out/no-such-dir/x.txt" "$log"
rm -f "$out/gone.txt"
./termshot --text "$out/gone.txt" "$log" "$out/gone.png" "$font" 255 500 73 2>/dev/null || true
check "failed run removes the text file it created" '[ ! -e "$out/gone.txt" ]'
# --json: the cursor, and runs of cells alike in colour and attributes.
printf '\033[1;31mab\033[m c\r\n\344\270\255x\033[?25l' | ./termshot --size 10x2 --json "$out/grid.json" -
printf '%s\n' '{"cols":10,"rows":2,"cursor":null,"lines":[' \
    '[{"col":0,"text":"ab","fg":"#cd0000","bg":"#111823","bold":true},{"col":2,"text":" c","fg":"#dbe7f7","bg":"#111823"}],' \
    '[{"col":0,"text":"中x","fg":"#dbe7f7","bg":"#111823"}]' ']}' > "$out/grid-want.json"
check "--json writes the runs and the cursor" 'cmp -s "$out/grid.json" "$out/grid-want.json"'
printf 'ab' | ./termshot --size 10x2 --cursor 4,1 --json - - > "$out/grid-cursor.json"
check "--json reports --cursor" 'grep -q "\"cursor\":{\"col\":4,\"row\":1,\"shape\":\"block\"}" "$out/grid-cursor.json"'
printf '\033[5 qab' | ./termshot --size 10x2 --json - - > "$out/grid-shape.json"
check "--json reports the shape the log sets" 'grep -q "\"cursor\":{\"col\":2,\"row\":0,\"shape\":\"bar\"}" "$out/grid-shape.json"'
printf '\033[5 qab' | ./termshot --size 10x2 --cursor-shape underline --json - - > "$out/grid-shape-option.json"
check "--json reports --cursor-shape" 'grep -q "\"shape\":\"underline\"" "$out/grid-shape-option.json"'
./termshot --text "$out/both.txt" --json "$out/both.json" "$log"
check "--text and --json together match each alone" 'cmp -s "$out/both.txt" "$out/text-png.txt"'
expect 2 --json - --text - "$log"
expect 2 --json - "$log" -
expect 1 --json "$out/no-such-dir/x.json" "$log"
# asciicast input: v2 and v3 replay their output events, sized by the
# recording (the header, or its last resize) unless a size is given.
cast2=tests/fixtures/asciicast-v2.cast
cast3=tests/fixtures/asciicast-v3.cast
./termshot --size 24x6 --text "$out/cast-raw.txt" tests/fixtures/asciicast.pty "$out/cast-raw.png"
./termshot --text "$out/cast-v2.txt" "$cast2" "$out/cast-v2.png"
./termshot --text "$out/cast-v3.txt" "$cast3" "$out/cast-v3.png"
check "a v2 cast renders as its raw output at its header's size" 'cmp -s "$out/cast-v2.png" "$out/cast-raw.png"'
check "a v3 cast renders as its raw output at its last resize" 'cmp -s "$out/cast-v3.png" "$out/cast-raw.png"'
check "a cast's --text is the raw output's" 'cmp -s "$out/cast-v2.txt" "$out/cast-raw.txt" && cmp -s "$out/cast-v3.txt" "$out/cast-raw.txt"'
./termshot - "$out/cast-stdin.png" < "$cast3"
check "a cast on stdin is detected" 'cmp -s "$out/cast-stdin.png" "$out/cast-raw.png"'
./termshot --cast "$cast2" "$out/cast-flag.png"
check "--cast reads it the same" 'cmp -s "$out/cast-flag.png" "$out/cast-raw.png"'
check "a cast is quiet on success" '[ -z "$(./termshot "$cast3" "$out/q.png" 2>&1)" ]'
./termshot --size 30x8 "$cast2" "$out/cast-size.png"
./termshot --size 30x8 tests/fixtures/asciicast.pty "$out/cast-raw-size.png"
check "--size overrides the cast's size" 'cmp -s "$out/cast-size.png" "$out/cast-raw-size.png"'
./termshot "$cast3" "$out/cast-legacy.png" "$font" 48 30 8
check "the original form's size overrides it too" 'cmp -s "$out/cast-legacy.png" "$out/cast-raw-size.png"'
./termshot --size 24x6 "$cast3" "$out/cast-cols.png" "$font" 48 2>/dev/null
check "--size and a font as the original form" 'cmp -s "$out/cast-cols.png" "$out/cast-raw.png"'
check "only cols given: rows come from the cast" \
    './termshot "$cast2" "$out/cast-c.png" "$font" 24 30 && ./termshot --size 30x6 tests/fixtures/asciicast.pty "$out/cast-c-raw.png" -p 24 && cmp -s "$out/cast-c.png" "$out/cast-c-raw.png"'
check "--json reports the cast's size" './termshot --json - "$cast3" | grep -q "^{\"cols\":24,\"rows\":6,"'
check "--cursor is checked against the cast's size" \
    './termshot --cursor 25,0 "$cast3" "$out/x.png" 2>&1 | grep -q "off the 24x6 grid"'
expect 2 --cursor 25,0 "$cast3" "$out/x.png"
expect 0 --cursor 24,5 "$cast3" "$out/x.png"
printf '{"version":2,"width":501,"height":10}\n[0,"o","a"]\n' > "$out/wide.cast"
printf '{"version":3,"term":{"cols":80,"rows":24}}\n[0,"r","80x201"]\n' > "$out/tall.cast"
printf '{"version":2,"width":0,"height":10}\n' > "$out/zero.cast"
expect 2 "$out/wide.cast" "$out/x.png"
expect 2 "$out/tall.cast" "$out/x.png"
expect 2 "$out/zero.cast" "$out/x.png"
expect 0 --size 10x4 "$out/wide.cast" "$out/x.png"
expect 0 --size 10x4 "$out/tall.cast" "$out/x.png"
check "an oversized cast says where its size came from" \
    './termshot "$out/tall.cast" "$out/x.png" 2>&1 | grep -q "201 rows (its last resize event).*pass --size"'
# A malformed cast is an unusable input: exit 1, with the line, and no output left behind.
printf '{"version":2,"width":10,"height":2}\n[0,"o","ok"]\n[0,"o","cut' > "$out/truncated.cast"
printf '{"version":2,"width":10,"height":2}\n[0,"o","\\ud800"]\n' > "$out/surrogate.cast"
printf '{"version":2,"width":10,"height":2}\n[0,"o","\377"]\n' > "$out/latin1.cast"
printf '{"version":1,"width":10,"height":2}\n' > "$out/v1.cast"
printf '{"version":2,"width":"10","height":2}\n' > "$out/types.cast"
printf '{"version":2,"width":10,"height":2}\n[0,"o",%s]\n' "$(printf '[%.0s' $(seq 20))" > "$out/deep.cast"
for bad in truncated surrogate latin1 v1 types deep; do
    expect 1 "$out/$bad.cast" "$out/x.png"
    rm -f "$out/gone.png" "$out/gone.txt"
    ./termshot --text "$out/gone.txt" "$out/$bad.cast" "$out/gone.png" 2>/dev/null || true
    check "a malformed cast ($bad) leaves no output" '[ ! -e "$out/gone.png" ] && [ ! -e "$out/gone.txt" ]'
done
check "a malformed cast names the line" \
    './termshot "$out/truncated.cast" "$out/x.png" 2>&1 | grep -q "truncated.cast: not a readable asciicast: line 3, column 12: the line ends inside a string"'
check "an unsupported version says so" './termshot "$out/v1.cast" "$out/x.png" 2>&1 | grep -q "version 1 is not supported"'
expect 1 --cast "$log" "$out/x.png"
expect 2 --cast=yes "$cast2" "$out/x.png"
# --raw: output that happens to start as a cast does is still a raw log.
printf '{"version":2,"msg":"hi"}\r\nok' > "$out/json-first.pty"
expect 1 "$out/json-first.pty" "$out/x.png"
check "a detected cast that fails suggests --raw" \
    './termshot "$out/json-first.pty" "$out/x.png" 2>&1 | grep -q "no width; if it is raw PTY output, pass --raw"'
check "--cast doesn't suggest --raw" '! ./termshot --cast "$out/json-first.pty" "$out/x.png" 2>&1 | grep -q -- --raw'
printf '{"version":2,"msg":"hi"}\nok\n' > "$out/json-first.want"
check "--raw reads it as raw output" \
    './termshot --raw --size 30x2 --text - "$out/json-first.pty" | cmp -s - "$out/json-first.want"'
./termshot --raw "$cast2" "$out/cast-as-raw.png"
check "--raw draws a cast's JSON, not its output" '! cmp -s "$out/cast-as-raw.png" "$out/cast-raw.png"'
expect 2 --cast --raw "$cast2" "$out/x.png"
expect 2 --raw --cast "$cast2" "$out/x.png"
expect 2 --raw=no "$cast2" "$out/x.png"
# Detection never takes a raw log for a cast: only a first-line JSON object with a version is one.
printf '{"width":3}\r\nplain' | ./termshot --size 12x2 - "$out/not-cast.png"
printf '{"width":3}\r\nplain' | ./termshot --size 12x2 --text - - > "$out/not-cast.txt"
check "a JSON first line without a version is a raw log" 'printf "{\"width\":3}\nplain\n" | cmp -s - "$out/not-cast.txt"'
check "the --lf-newline hint looks at a cast's output" \
    'printf "{\"version\":2,\"width\":10,\"height\":2}\n[0,\"o\",\"a\\\\nb\"]\n" | ./termshot - "$out/q.png" 2>&1 | grep -q -- "stdin has line feeds but no CR"'
check "a cast from a PTY gives no hint, though its lines end in LF" '[ -z "$(./termshot "$cast2" "$out/q.png" 2>&1)" ]'
# No output may be another output or an input, however the path is spelled.
cp "$log" "$out/clash.pty"
ln -sf clash.pty "$out/clash-link.pty"
rm -f "$out/clash.txt"
expect 2 --text "$out/clash.txt" --json "$out/clash.txt" "$log"
expect 2 --text "$out/clash.txt" --json "$out/../test/clash.txt" "$log"
expect 2 --json "$out/clash.txt" "$log" "$out/clash.txt"
check "a refused clash creates no file" '[ ! -e "$out/clash.txt" ]'
expect 2 --text "$out/clash.pty" "$out/clash.pty"
expect 2 --json "$out/clash-link.pty" "$out/clash.pty"
expect 2 "$out/clash.pty" "./$out/clash.pty"
expect 2 --font "$out/clash.pty" "$log" "$out/clash.pty"
check "a clash leaves the input as it was" 'cmp -s "$out/clash.pty" "$log"'
# --palette, --fg, --bg and --padding (#87). tests/palette_padding.rs checks
# their pixels; here, their values, the file's errors and the grid outputs.
pal=$out/cli-palette.conf
printf '# a theme\nforeground #102030\nbackground #f0e0d0\ncolor1 #00ff00\n' > "$pal"
for bad in -1 1025 1,2,3 1, ,1 x 1.5 '1 2' 0x10; do
    expect 2 "$log" "$out/x.png" --padding "$bad"
done
expect 2 "$log" "$out/x.png" --padding
expect 0 "$log" "$out/x.png" --padding 1024,0
check "--padding names its range and form" \
    './termshot --padding 2000 "$log" "$out/x.png" 2>&1 | grep -q "padding must be pixels from 0 to 1024, as 16 or 32,16"'
for bad in red '#fff' '#12345g' 123456 '#1234567' ''; do
    expect 2 "$log" "$out/x.png" --fg "$bad"
    expect 2 "$log" "$out/x.png" --bg "$bad"
done
expect 2 "$log" "$out/x.png" --fg
expect 2 "$log" "$out/x.png" --palette
check "--bg names the colour form" './termshot --bg red "$log" "$out/x.png" 2>&1 | grep -q -- "--bg: a colour must look like #1e2a3b, not \"red\""'
expect 1 "$log" "$out/x.png" --palette "$out/no-such.conf"
expect 1 "$log" "$out/x.png" --palette "$out"
printf 'color1 #00ff00\ncursor #ffffff\n' > "$out/bad-palette.conf"
expect 2 "$log" "$out/x.png" --palette "$out/bad-palette.conf"
check "a malformed palette names the file, the line and the key" \
    './termshot --palette "$out/bad-palette.conf" "$log" "$out/x.png" 2>&1 | grep -q -- "--palette $out/bad-palette.conf: line 2: unknown key \"cursor\""'
printf 'color16 #ffffff\n' > "$out/bad-palette.conf"
check "a palette refuses colours past the 16" \
    './termshot --palette "$out/bad-palette.conf" "$log" "$out/x.png" 2>&1 | grep -q "color16 is not one of the 16 named colours"'
head -c 70000 /dev/zero | tr '\0' '#' > "$out/big-palette.conf"
expect 2 "$log" "$out/x.png" --palette "$out/big-palette.conf"
for bad in bad big; do
    rm -f "$out/gone.png" "$out/gone.txt"
    ./termshot --palette "$out/$bad-palette.conf" --text "$out/gone.txt" "$log" "$out/gone.png" 2>/dev/null || true
    check "a refused palette ($bad) leaves no output" '[ ! -e "$out/gone.png" ] && [ ! -e "$out/gone.txt" ]'
done
check "a palette is checked before the log is read" \
    'printf x | ./termshot --palette "$out/bad-palette.conf" - "$out/x.png" 2>&1 | grep -q "color16"'
cp "$pal" "$out/clash.conf"
expect 2 --palette "$out/clash.conf" "$log" "$out/clash.conf"
check "a palette named as an output is kept" 'cmp -s "$out/clash.conf" "$pal"'
# A palette or font named - is that file, not stdin, so an output naming it clashes too.
cp "$pal" "$out/-"
check "a palette named - is a file an output may not overwrite" \
    '(cd "$out" && ../../termshot --palette - "../../$log" ./- 2>&1 | grep -q "<out.png> ./- is the --palette file") && cmp -s "$out/-" "$pal"'
cp "$font" "$out/-"
check "a font named - is a file an output may not overwrite" \
    '(cd "$out" && ../../termshot --font - --text ./- "../../$log" 2>&1 | grep -q "is the --font file") && cmp -s "$out/-" "$font"'
rm -f "$out/-"
check "a palette and padding are quiet" '[ -z "$(./termshot --palette "$pal" --fg "#ffffff" --padding 8,4 "$log" "$out/q.png" 2>&1)" ]'
check "-v prints the padded image size" \
    './termshot -v --padding 16,10 "$log" "$out/q.png" 2>&1 | grep -q "image 2232x1460"'
# The padding counts towards the image limit: 11000x9600 fits, with 1024 on each side it does not.
rm -f "$out/gone.png"
expect 2 --size 500x200 --padding 1024 "$log" "$out/gone.png"
check "a padded image over the limit says so, and leaves no output" \
    './termshot --size 500x200 --padding 1024 "$log" "$out/gone.png" 2>&1 | grep -q "image 13048x11648 is over 134217728 pixels; lower px, cols, rows or padding" && [ ! -e "$out/gone.png" ]'
# --json reports the colours the palette resolves to; --fg and --bg override
# the file's; padding changes no grid output.
printf '\033[31ma\033[m b\033[41m \033[m\033[?25l' | ./termshot --size 6x1 --palette "$pal" --bg '#000001' --json - - > "$out/grid-palette.json"
printf '%s\n' '{"cols":6,"rows":1,"cursor":null,"lines":[' \
    '[{"col":0,"text":"a","fg":"#00ff00","bg":"#000001"},{"col":1,"text":" b","fg":"#102030","bg":"#000001"},{"col":3,"text":" ","fg":"#102030","bg":"#00ff00"}]' \
    ']}' > "$out/grid-palette-want.json"
check "--json reports the palette's colours" 'cmp -s "$out/grid-palette.json" "$out/grid-palette-want.json"'
./termshot --text "$out/pad.txt" --json "$out/pad.json" --padding 40 "$log" "$out/pad.png"
check "padding changes no --text or --json" 'cmp -s "$out/pad.txt" "$out/text-png.txt" && cmp -s "$out/pad.json" "$out/both.json"'
check "--padding without a PNG needs no font" './termshot --font README.md --padding 4 --text "$out/q.txt" "$log"'
# A wide character on a one-column screen (#18).
check "one-column wide character renders" 'printf "\347\225\214" | ./termshot --size 1x1 - "$out/one-column.png"'
# Quiet unless asked; a failed run leaves no file behind.
check "quiet by default" '[ -z "$(./termshot "$log" "$out/q.png" 2>&1)" ]'
check "a log with LF but no CR hints at --lf-newline" 'printf "a\nb" | ./termshot - "$out/q.png" 2>&1 | grep -q -- "pass --lf-newline"'
check "the hint names the file" 'printf "a\nb" > "$out/bare.log" && ./termshot "$out/bare.log" "$out/q.png" 2>&1 | grep -q "bare.log has line feeds"'
check "no hint with --lf-newline" '[ -z "$(printf "a\nb" | ./termshot --lf-newline - "$out/q.png" 2>&1)" ]'
check "no hint for CR LF" '[ -z "$(printf "a\r\nb" | ./termshot - "$out/q.png" 2>&1)" ]'
check "the hint keeps exit status 0" 'printf "a\nb" | ./termshot - "$out/q.png" 2>/dev/null'
check "verbose prints the image size" './termshot -v "$log" "$out/q.png" 2>&1 | grep -q "image 2200x1440"'
rm -f "$out/gone.png"
./termshot "$log" "$out/gone.png" "$font" 255 500 73 2>/dev/null || true
check "failed run removes the file it created" '[ ! -e "$out/gone.png" ]'
if [ -e /dev/full ]; then
    check "short writes are reported" '! ./termshot "$log" /dev/full 2>/dev/null'
    check "short text writes are reported" '! ./termshot --text /dev/full "$log" 2>/dev/null'
fi
[ "$fail" -eq 0 ] && echo "ok"

echo "== goldens"
rustc --edition 2021 tests/golden.rs -o "$out/golden" -C opt-level=2 \
    -L native="$PWD/$out" -l static=png_read
if [ "$mode" = update ]; then
    "$out/golden" --update
    # The unit tests checked the grids before they were rewritten.
    "$out/unit" -q grid_json || fail=1
else
    "$out/golden" || fail=1
fi
echo "== the library, linked as an embedder links it"
# After the goldens, which check tests/grids/ against the CLI: the library
# must write the same grids.
# shellcheck disable=SC2086
rustc --edition 2021 tests/library.rs -o "$out/library" --extern termshot=libtermshot.rlib \
    -L native="$PWD/$out" -l static=png_read ${RUSTC_LINK_ARGS:-}
"$out/library" || fail=1
echo "== the crate docs' examples"
# They are no_run (they read session.pty), so rustdoc compiles and links
# each against the rlib without running it.
# shellcheck disable=SC2086
if rustdoc --edition 2021 --test src/lib.rs --crate-name termshot --extern termshot=libtermshot.rlib \
    ${RUSTC_LINK_ARGS:-} > "$out/doctests.log" 2>&1; then
    echo "ok, $(grep -c '\.\.\. ok$' "$out/doctests.log") examples compile"
else
    cat "$out/doctests.log"
    fail=1
fi
echo "== kitty graphics pixels"
rustc --edition 2021 tests/graphics.rs -o "$out/graphics" -L native="$out" -l static=png_read
"$out/graphics"
echo "== palette and padding pixels"
rustc --edition 2021 tests/palette_padding.rs -o "$out/palette_padding" -L native="$out" -l static=png_read
"$out/palette_padding"

# Exercise the production PNG decoder, including allocation quota failures.
# shellcheck disable=SC2086
cc tests/image.c "$rust" -I third_party/stb -O2 -Wno-unused-function ${CFLAGS:-} $rust_libs -o "$out/image"
"$out/image"

echo "== output aliases"
rustc --edition 2021 tests/output_aliases.rs -o "$out/output_aliases"
"$out/output_aliases"

echo "== font messages"
rustc --edition 2021 tests/font_cli.rs -o "$out/font_cli"
"$out/font_cli" ./termshot "$out/hollow-A.ttf" || fail=1

echo "== generated files are what their generators write"
rustc --edition 2021 -O tools/crc32-table.rs -o "$out/crc32-table"
"$out/crc32-table" > "$out/crc32_table.h"
check "src/crc32_table.h is what tools/crc32-table.rs writes" 'cmp -s "$out/crc32_table.h" src/crc32_table.h'
rustc --edition 2021 -O scripts/perf-fixtures.rs -o "$out/perf-fixtures"
check "tests/perf/ is what scripts/perf-fixtures.rs writes" '"$out/perf-fixtures" --check >/dev/null'
# The CFF POC's fonts (src/cff_craft.rs, which the unit tests use too):
# the control draws, a defect is refused.
rustc --edition 2021 bench/cff-poc/craft.rs -o "$out/craft-fonts"
mkdir -p "$out/craft"
"$out/craft-fonts" "$out/craft"
check "bench/cff-poc/craft.rs writes the POC's nine fonts" '[ "$(ls "$out/craft"/*.otf | wc -l)" -eq 9 ]'
check "its control font draws" 'printf A | ./termshot --font "$out/craft/control.otf" - "$out/craft.png"'
expect 1 --font "$out/craft/bad_offsize.otf" "$log" "$out/craft.png"

exit "$fail"
