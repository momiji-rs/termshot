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
# The pipeline it is for: tmux capture-pane of a pane the size of the grid.
# Needs tmux, which CI doesn't have.
if command -v tmux >/dev/null; then
    socket="termshot-test-$$"
    printf '\033[31mred\033[m\r\nplain\r\n\r\nlast' > "$out/pane.pty"
    tmux -L "$socket" -f /dev/null new-session -d -x 10 -y 4 "cat '$out/pane.pty'; sleep 30"
    tries=0
    until tmux -L "$socket" capture-pane -p | grep -q last || [ "$tries" -ge 50 ]; do
        sleep 0.1
        tries=$((tries + 1))
    done
    tmux -L "$socket" capture-pane -e -p | ./termshot --lf-newline --size 10x4 - "$out/pane.png"
    tmux -L "$socket" kill-server
    ./termshot --size 10x4 "$out/pane.pty" "$out/pane-direct.png"
    check "tmux capture-pane renders like the bytes the pane was sent" 'cmp -s "$out/pane.png" "$out/pane-direct.png"'
else
    echo "skip: tmux capture-pane check (no tmux)"
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
