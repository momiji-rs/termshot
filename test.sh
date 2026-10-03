#!/bin/sh
# Build, then run the parser unit tests, the CLI exit-code checks and the
# pixel goldens. Renders land in target/test/ and are kept for inspection.
#
#   ./test.sh                    run everything
#   ./test.sh --update-goldens   rewrite tests/goldens.txt from this build
#   SANITIZE=1 ./test.sh         build draw.c with ASan + UBSan (macOS clang)
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
    export ASAN_OPTIONS=detect_leaks=0:abort_on_error=1
fi

./build.sh

echo "== unit tests"
# shellcheck disable=SC2086
rustc --edition 2021 --test src/main.rs -o "$out/unit" \
    -L native="$PWD" -l static=termshot_c ${RUSTC_LINK_ARGS:-}
"$out/unit" -q

echo "== deflate matches stb"
# shellcheck disable=SC2086
cc tests/deflate_diff.c src/deflate.c -o "$out/deflate_diff" -O2 -Wno-deprecated-declarations \
    -I third_party/stb ${CFLAGS:-}
"$out/deflate_diff"

echo "== box drawing and blocks"
# shellcheck disable=SC2086
cc tests/boxes.c src/deflate.c -o "$out/boxes" -O2 -ffp-contract=off -Wno-deprecated-declarations \
    -I src -I third_party/stb -lm ${CFLAGS:-}
"$out/boxes"

echo "== glyph placement"
# The decoder is test-only, so it is built without sanitizers or our warnings.
cc -c tests/png_read.c -o "$out/png_read.o" -O2 -I third_party/stb
rm -f "$out/libpng_read.a"
ar rcs "$out/libpng_read.a" "$out/png_read.o"
# shellcheck disable=SC2086
cc tests/glyphs.c src/deflate.c "$out/png_read.o" -o "$out/glyphs" -O2 -Wno-deprecated-declarations \
    -I src -I third_party/stb -lm ${CFLAGS:-}
"$out/glyphs" "$font" "$out/glyphs.png"

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
# --text: the screen as text, with the PNG or without it.
printf 'ab\r\ncd  \r\n\344\270\255x' | ./termshot --size 10x4 --text "$out/text.txt" -
printf 'ab\ncd\n\344\270\255x\n\n' > "$out/text-want.txt"
check "--text alone writes the rows, trimmed" 'cmp -s "$out/text.txt" "$out/text-want.txt"'
printf 'ab\r\ncd  \r\n\344\270\255x' | ./termshot --size 10x4 --text - - > "$out/text-stdout.txt"
check "--text - writes stdout" 'cmp -s "$out/text-stdout.txt" "$out/text-want.txt"'
./termshot --text "$out/text-png.txt" "$log" "$out/text-png.png"
check "--text leaves the PNG as it was" 'cmp -s "$out/text-png.png" "$out/builtin.png"'
check "--text needs no font without a PNG" './termshot --font README.md --text "$out/q.txt" "$log"'
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
check "--json reports --cursor" 'grep -q "\"cursor\":{\"col\":4,\"row\":1}" "$out/grid-cursor.json"'
./termshot --text "$out/both.txt" --json "$out/both.json" "$log"
check "--text and --json together match each alone" 'cmp -s "$out/both.txt" "$out/text-png.txt"'
expect 2 --json - --text - "$log"
expect 2 --json - "$log" -
expect 1 --json "$out/no-such-dir/x.json" "$log"
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
# Every golden's JSON parses, and its runs spell the --text rows.
if command -v python3 >/dev/null; then
    check "the golden JSON parses and agrees with --text" 'python3 tests/grids/check.py tests/grids'
else
    echo "skip: golden JSON check (no python3)"
fi
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
else
    "$out/golden" || fail=1
fi
exit "$fail"
