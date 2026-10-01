#!/bin/sh
# Compare tests/vt/expected.txt against tmux, a full VT emulator, as an
# independent reference for termshot's parser. Needs tmux; not run in CI
# (the unit tests check termshot against expected.txt without it).
#
#   tests/vt/oracle.sh            report cases where tmux differs from expected.txt
#   tests/vt/oracle.sh --update   rewrite expected.txt from tmux, keeping the
#                                 hand-written screens of cases in deviations.txt
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

# The screen tmux shows for one case: "== name", then one line per row with
# trailing spaces trimmed.
tmux_screen() {
    name=$1 size=$2 input=$3
    printf "$input" > "$work/log"
    tmux -L "$socket" kill-server 2>/dev/null || true
    # -onlcr keeps a bare LF a bare LF, as in a captured PTY stream.
    tmux -L "$socket" -f /dev/null new-session -d -x "${size%x*}" -y "${size#*x}" \
        "stty -onlcr; cat '$work/log'; sleep 5"
    sleep 0.3
    echo "== $name"
    tmux -L "$socket" capture-pane -p -t 0 | sed 's/[[:space:]]*$//'
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
    tmux_screen "$name" "$size" "$input" > "$work/one.txt"
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
    exit 0
fi

status=0
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
[ -f "$work/failed" ] && status=1
exit $status
