#!/bin/sh
# Stage the npm packages from the release archives, one directory each:
#
#   scripts/npm-stage.sh <version> <archive-dir> <out-dir>
#
# @momiji-rs/termshot holds npm/termshot.js, which runs the binary of the
# platform package npm picked by os and cpu from its optionalDependencies:
# @momiji-rs/termshot-darwin-universal, -linux-x64 and -linux-arm64. Each of
# those holds one release binary as it is, so npm installs run the bytes the
# GitHub release checked. release.yml packs and publishes the directories.
set -eu
usage='usage: scripts/npm-stage.sh <version> <archive-dir> <out-dir>'
version=${1:?$usage}
archives=$(cd "${2:?$usage}" && pwd)
out=${3:?$usage}
rm -rf "$out"
mkdir -p "$out"
out=$(cd "$out" && pwd)
cd "$(dirname "$0")/.."
repo='{ "type": "git", "url": "git+https://github.com/momiji-rs/termshot.git" }'

# platform package name, release archive platform, npm os, npm cpu
for spec in darwin-universal:macos-universal:'"darwin"':'"arm64", "x64"' \
            linux-x64:linux-x86_64-musl:'"linux"':'"x64"' \
            linux-arm64:linux-aarch64-musl:'"linux"':'"arm64"'; do
    name=${spec%%:*}; rest=${spec#*:}
    platform=${rest%%:*}; rest=${rest#*:}
    os=${rest%%:*}; cpu=${rest#*:}
    dir="$out/termshot-$name"
    mkdir -p "$dir"
    tar -xzf "$archives/termshot-$version-$platform.tar.gz" -C "$dir" --strip-components 1 \
        "termshot-$version-$platform/termshot" \
        "termshot-$version-$platform/LICENSE" \
        "termshot-$version-$platform/LICENSE-JetBrains-Mono.txt"
    # Only this host's binary runs here; release.sh checked each on its own runner.
    if got=$("$dir/termshot" --version 2>/dev/null) && [ "$got" != "termshot $version" ]; then
        echo "$platform: the binary says '$got', not termshot $version" >&2
        exit 1
    fi
    cat > "$dir/package.json" <<JSON
{
  "name": "@momiji-rs/termshot-$name",
  "version": "$version",
  "description": "The termshot binary for $name. Install @momiji-rs/termshot instead.",
  "license": "MIT",
  "repository": $repo,
  "os": [$os],
  "cpu": [$cpu],
  "files": ["termshot", "LICENSE", "LICENSE-JetBrains-Mono.txt"],
  "publishConfig": { "access": "public" }
}
JSON
    printf '# @momiji-rs/termshot-%s\n\nThe [termshot](https://github.com/momiji-rs/termshot) %s binary for %s. Install [@momiji-rs/termshot](https://www.npmjs.com/package/@momiji-rs/termshot); it depends on this package.\n' \
        "$name" "$version" "$name" > "$dir/README.md"
done

dir="$out/termshot"
mkdir -p "$dir/bin"
cp npm/termshot.js "$dir/bin/termshot.js"
chmod 755 "$dir/bin/termshot.js"
cp LICENSE "$dir/LICENSE"
cat > "$dir/package.json" <<JSON
{
  "name": "@momiji-rs/termshot",
  "version": "$version",
  "description": "A screenshot tool for terminal output you already have: replays a PTY log or asciinema cast into a PNG, text or JSON of the final screen, headless.",
  "keywords": ["terminal", "screenshot", "tui", "pty", "ansi", "asciinema", "png"],
  "homepage": "https://github.com/momiji-rs/termshot",
  "license": "MIT",
  "repository": $repo,
  "bin": { "termshot": "bin/termshot.js" },
  "files": ["bin/termshot.js", "LICENSE"],
  "engines": { "node": ">=22" },
  "optionalDependencies": {
    "@momiji-rs/termshot-darwin-universal": "$version",
    "@momiji-rs/termshot-linux-x64": "$version",
    "@momiji-rs/termshot-linux-arm64": "$version"
  },
  "publishConfig": { "access": "public" }
}
JSON
cat > "$dir/README.md" <<MD
# @momiji-rs/termshot

[termshot](https://github.com/momiji-rs/termshot) $version replays a raw PTY log or an asciinema
\`.cast\` and writes the final screen as a PNG, text or JSON. It runs headless and gives the same
pixels on macOS and Linux.

\`\`\`sh
npx -y @momiji-rs/termshot session.pty session.png
npm install -g @momiji-rs/termshot   # then: termshot --help
\`\`\`

This package runs the release binary for macOS (arm64, x86_64) or Linux (x86_64, arm64), which
npm installs as an optional dependency. Other platforms: see the
[README](https://github.com/momiji-rs/termshot#install).
MD
echo "staged $version in $out:"
ls "$out"
