# Sourced by oracle.sh and record.sh: the last line of a screen block.
#
#   cursor_line <socket> <cols>   prints "cursor COL,ROW" (from 0) or "cursor hidden"
#
# With a wrap pending, tmux reports the cursor one column past the last; it
# is drawn on the last column, as termshot keeps it, so the column is clamped.
# A shape DECSCUSR set, other than a block, follows: "cursor 3,0 bar".
cursor_line() {
    tmux -L "$1" display -p -t 0 '#{cursor_x} #{cursor_y} #{cursor_flag} #{cursor_shape}' | {
        read -r x y shown shape
        [ "$x" -ge "$2" ] && x=$(($2 - 1))
        case $shape in underline | bar) shape=" $shape" ;; *) shape= ;; esac
        if [ "$shown" = 1 ]; then echo "cursor $x,$y$shape"; else echo "cursor hidden"; fi
    }
}
