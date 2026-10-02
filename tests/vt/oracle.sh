#!/bin/sh
# Compare tests/vt/expected.txt against tmux, a full VT emulator, as an
# independent reference for termshot's parser: the screen of each case, and
# where its cursor is. Also replays each real/<name>.log and compares the
# result with real/<name>.txt. Needs tmux; not run in CI (the unit tests
# check termshot against expected.txt and real/ without it).
#
#   tests/vt/oracle.sh            report cases where tmux differs from expected.txt
#   tests/vt/oracle.sh --update   rewrite expected.txt from tmux, keeping the
#                                 hand-written screens of cases in deviations.txt,
#                                 and add the cursor to real/<name>.txt files
#                                 recorded without one
#
# Checked with tmux 3.7c.
set -eu
cd "$(dirname "$0")"
command -v tmux >/dev/null || { echo "tmux not found" >&2; exit 2; }
mode=check
[ "${1:-}" = --update ] && mode=update
work=$(mktemp -d)
socket="termshot-oracle-$$"
trap 'tmux -L "$socket" kill-server 2>/dev/null; rm -rf "$work"' EXIT INT TERM
. ./cursor.sh

# The screen tmux shows after the bytes in a file: one line per row with
# trailing spaces trimmed, then the cursor line.
tmux_screen() {
    size=$1 log=$2
    tmux -L "$socket" kill-server 2>/dev/null || true
    # A fresh server each time: a new session on the socket of one that is
    # still shutting down can die with it ("server exited unexpectedly").
    runs=$((${runs:-0} + 1))
    socket="termshot-oracle-$$-$runs"
    # -onlcr keeps a bare LF a bare LF, as in a captured PTY stream. -echo
    # keeps tmux's answers to queries in the log (vi asks where the cursor
    # is) off the screen.
    tmux -L "$socket" -f /dev/null new-session -d -x "${size%x*}" -y "${size#*x}" \
        "stty -onlcr -echo; cat '$log'; sleep 5"
    sleep "${settle:-0.3}"
    tmux -L "$socket" capture-pane -p -t 0 | sed 's/[[:space:]]*$//'
    cursor_line "$socket" "${size%x*}"
}

# One case's block from a screens file.
block() {
    awk -v name="$1" '$0 == "== " name { on = 1; print; next } /^== / { on = 0 } on' "$2"
}

deviates() {
    grep -q "^$1:" deviations.txt
}

: > "$work/tmux.txt"
: > "$work/new.txt"
grep -v '^#' cases.txt | grep -v '^[[:space:]]*$' | while read -r name size input; do
    printf "$input" > "$work/log"
    { echo "== $name"; tmux_screen "$size" "$work/log"; } > "$work/one.txt"
    cat "$work/one.txt" >> "$work/tmux.txt"
    if deviates "$name" && block "$name" expected.txt | grep -q .; then
        block "$name" expected.txt >> "$work/new.txt"
    else
        cat "$work/one.txt" >> "$work/new.txt"
    fi
done

if [ "$mode" = update ]; then
    cp "$work/new.txt" expected.txt
    echo "updated tests/vt/expected.txt"
else
    grep -v '^#' cases.txt | grep -v '^[[:space:]]*$' | while read -r name size input; do
        want=$(block "$name" expected.txt)
        got=$(block "$name" "$work/tmux.txt")
        if [ "$want" = "$got" ]; then
            deviates "$name" && echo "note: $name now matches tmux; drop it from deviations.txt"
        elif deviates "$name"; then
            echo "documented: $name ($(grep "^$name:" deviations.txt | cut -d: -f2- | sed 's/^ *//'))"
        else
            echo "FAIL: $name: tmux shows"
            printf '%s\n' "$got" | sed 's/^/    |/'
            echo "  expected.txt has"
            printf '%s\n' "$want" | sed 's/^/    |/'
            echo fail >> "$work/failed"
        fi
        : # the loop's status is its last command's; keep set -e from tripping on it
    done
fi

# The real sessions: replaying a log reproduces the screen and cursor it was
# recorded with.
for log in real/*.log; do
    txt=${log%.log}.txt
    settle=1 tmux_screen 80x24 "$PWD/$log" > "$work/real.txt"
    if [ "$mode" = update ] && ! grep -q '^cursor ' "$txt"; then
        # Recorded before record.sh wrote the cursor line: take it from the replay.
        tail -n 1 "$work/real.txt" >> "$txt"
        echo "added the cursor to tests/vt/$txt"
    elif ! cmp -s "$work/real.txt" "$txt"; then
        echo "FAIL: $log replayed in tmux differs from $txt:"
        diff "$txt" "$work/real.txt" | sed 's/^/    /' || true
        echo fail >> "$work/failed"
    fi
done
[ -f "$work/failed" ] && exit 1
exit 0
