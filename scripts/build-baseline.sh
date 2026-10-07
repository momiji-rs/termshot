#!/bin/sh
# Build a reference termshot from an older revision, optionally also with a
# profiling patch, without switching or modifying the worktree.
#
#   scripts/build-baseline.sh DESTINATION [--revision REV] [--profile-patch PATCH]
#
# DESTINATION must not exist. The reference is DESTINATION/original, the
# patched build DESTINATION/baseline. REV defaults to c44d83c.
set -eu
usage="usage: scripts/build-baseline.sh DESTINATION [--revision REV] [--profile-patch PATCH]"
root=$(cd "$(dirname "$0")/.." && pwd)
dest= revision=c44d83c patch=
while [ $# -gt 0 ]; do
    case $1 in
        --revision) [ $# -ge 2 ] || { echo "$usage" >&2; exit 2; }; revision=$2; shift 2 ;;
        --revision=*) revision=${1#*=}; shift ;;
        --profile-patch) [ $# -ge 2 ] || { echo "$usage" >&2; exit 2; }; patch=$2; shift 2 ;;
        --profile-patch=*) patch=${1#*=}; shift ;;
        -h|--help) echo "$usage"; exit 0 ;;
        -*) echo "$usage" >&2; exit 2 ;;
        *) [ -z "$dest" ] || { echo "$usage" >&2; exit 2; }; dest=$1; shift ;;
    esac
done
[ -n "$dest" ] || { echo "$usage" >&2; exit 2; }
[ -z "$patch" ] || patch=$(cd "$(dirname "$patch")" && pwd -P)/$(basename "$patch")
[ ! -e "$dest" ] || { echo "$dest already exists" >&2; exit 1; }
mkdir -p "$dest"
dest=$(cd "$dest" && pwd -P)
# Read the revision's build inputs from git, one file at a time. The list
# goes through a file so a bad revision stops the script.
git -C "$root" ls-tree -r --name-only "$revision" -- build.sh src third_party/stb third_party/jetbrains-mono \
    > "$dest/.files" || exit 1
while IFS= read -r name; do
    mkdir -p "$dest/$(dirname "$name")"
    git -C "$root" show "$revision:$name" > "$dest/$name"
done < "$dest/.files"
rm "$dest/.files"
(cd "$dest" && sh build.sh)
mv "$dest/termshot" "$dest/original"
echo "Reference: $dest/original"
if [ -n "$patch" ]; then
    (cd "$dest" && patch -p1 -i "$patch" && sh build.sh)
    mv "$dest/termshot" "$dest/baseline"
    echo "Instrumented: $dest/baseline"
fi
