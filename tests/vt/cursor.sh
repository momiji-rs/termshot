# Sourced by oracle.sh and record.sh: the last line of a screen block.
#
#   cursor_line <socket> <cols>   prints "cursor COL,ROW" (from 0) or "cursor hidden"
#
# With a wrap pending, tmux reports the cursor one column past the last; it
# is drawn on the last column, as termshot keeps it, so the column is clamped.
cursor_line() {
    tmux -L "$1" display -p -t 0 '#{cursor_x} #{cursor_y} #{cursor_flag}' | {
        read -r x y shown
        [ "$x" -ge "$2" ] && x=$(($2 - 1))
        if [ "$shown" = 1 ]; then echo "cursor $x,$y"; else echo "cursor hidden"; fi
    }
}
