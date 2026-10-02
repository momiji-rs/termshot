#!/bin/sh
# Record a real program session in an 80x24 tmux pane: the raw bytes the
# programs wrote (pipe-pane) go to real/<name>.log, and the screen tmux shows
# at the end to real/<name>.txt, followed by where the cursor is. The unit
# tests check that termshot renders each log to that screen and cursor.
# Needs tmux; the sessions use macOS (BSD) tools.
#
#   tests/vt/record.sh shell|less|vi
set -eu
cd "$(dirname "$0")"
name=$1
socket="termshot-record-$$"
trap 'tmux -L "$socket" kill-server 2>/dev/null' EXIT INT TERM
. ./cursor.sh
rm -f "real/$name.log" "real/$name.txt"
tmux -L "$socket" -f /dev/null new-session -d -x 80 -y 24 \
    "sleep 1; exec env -i HOME=/tmp PATH=/usr/bin:/bin TERM=xterm-256color PS1='\$ ' LC_ALL=en_US.UTF-8 /bin/sh -i"
tmux -L "$socket" pipe-pane -o -t 0 "cat >> '$PWD/real/$name.log'"
sleep 1.5
keys() {
    tmux -L "$socket" send-keys -t 0 "$@"
    sleep "${pause:-0.6}"
}
case $name in
shell)
    # Colours, scrolling past the bottom, and a 250-character line that wraps.
    keys 'ls -G /' Enter
    keys 'seq 1 30' Enter
    keys "printf '%.0s0123456789' \$(seq 1 25); echo" Enter
    keys 'echo done' Enter
    ;;
less)
    # The alternate screen, entered and left.
    keys 'ls -G /usr' Enter
    keys 'less /etc/services' Enter
    keys ' '
    keys ' '
    keys q
    keys 'echo after-less' Enter
    ;;
vi)
    # Ends on the alternate screen, mid-edit.
    keys 'echo before-vi' Enter
    pause=1 keys 'vi -u NONE -c "set nocp" /etc/services' Enter
    keys 40G
    keys o
    keys 'typed in vi'
    keys Escape
    ;;
*)
    echo "unknown session: $name" >&2
    exit 2
    ;;
esac
sleep 1
tmux -L "$socket" capture-pane -p -t 0 | sed 's/[[:space:]]*$//' > "real/$name.txt"
cursor_line "$socket" 80 >> "real/$name.txt"
echo "recorded tests/vt/real/$name.log and $name.txt"
