#!/bin/sh
# Build one release archive into dist/ and check it before packing:
#
#   scripts/release.sh macos-universal       arm64 + x86_64 via lipo (macOS host)
#   scripts/release.sh linux-x86_64-musl     static, musl-gcc (x86_64 Linux host)
#   scripts/release.sh linux-aarch64-musl    static, musl-gcc (aarch64 Linux host)
#
# Each binary must render the samples byte for byte like the host build that
# ./test.sh checks against the goldens; musl's libm is not glibc's, so this is
# not a formality. With RELEASE_TAG set (v1.2.3), --version must match it.
set -eu
cd "$(dirname "$0")/.."
platform=${1:?usage: scripts/release.sh macos-universal|linux-x86_64-musl|linux-aarch64-musl}
work=target/release
rm -rf "$work"
mkdir -p "$work" dist

echo "== host build and reference renders"
./build.sh
for log in reply-sent draft-ready; do
    ./termshot "examples/$log.pty" "$work/$log.png"
done
mv termshot "$work/host"

# The release profile: build.sh's flags plus fat LTO, a 5-9% smaller
# binary. reply-sent is as fast; cursor-moves and two others are 1.5-3%
# slower, thai-combining 2-6% faster. docs/performance.md (#83) has every
# candidate measured and why the others are out: codegen-units=1 slows the
# parser 5-10%, lto=thin up to 6%, opt-level s and z nearly everything (up
# to 3x), and C -Os the CJK and glyph-heavy cases up to 3% on Linux. Not
# panic=abort: a panic at an FFI boundary is caught (exit 2, or an outline
# or advance recovered), and under panic=abort it would abort instead.
build() {
    # A caller's RUSTFLAGS are kept, as in the host build.
    RUSTFLAGS="-C lto=fat ${RUSTFLAGS:-}" ./build.sh
    mv termshot "$work/$1"
}

echo "== release build for $platform"
case $platform in
    macos-universal)
        export MACOSX_DEPLOYMENT_TARGET=11.0
        CFLAGS='-arch arm64' TARGET=aarch64-apple-darwin build arm64
        CFLAGS='-arch x86_64' TARGET=x86_64-apple-darwin build x86_64
        lipo -create "$work/arm64" "$work/x86_64" -output "$work/termshot"
        strip -x "$work/termshot"
        # One arch per check: Xcode 26's lipo takes a second one for an input file.
        lipo "$work/termshot" -verify_arch arm64
        lipo "$work/termshot" -verify_arch x86_64
        ;;
    linux-x86_64-musl | linux-aarch64-musl)
        arch=${platform#linux-}
        arch=${arch%-musl}
        CC=musl-gcc TARGET="$arch-unknown-linux-musl" build termshot
        strip "$work/termshot"
        file "$work/termshot" | grep -q 'statically linked\|static-pie linked' || { file "$work/termshot"; exit 1; }
        ;;
    *)
        echo "unknown platform: $platform" >&2
        exit 2
        ;;
esac

echo "== checks"
runs() {
    for log in reply-sent draft-ready; do
        "$@" "examples/$log.pty" "$work/check.png"
        cmp "$work/check.png" "$work/$log.png"
    done
    echo "renders match the host build: $*"
}
runs "$work/termshot"
# The x86_64 slice runs only where Rosetta is installed.
if [ "$platform" = macos-universal ] && arch -x86_64 /usr/bin/true 2>/dev/null; then
    runs arch -x86_64 "$work/termshot"
fi
version=$("$work/termshot" --version | sed 's/^termshot //')
if [ -n "${RELEASE_TAG:-}" ] && [ "$RELEASE_TAG" != "v$version" ]; then
    echo "tag $RELEASE_TAG does not match termshot $version" >&2
    exit 1
fi

name=termshot-$version-$platform
mkdir "$work/$name"
cp "$work/termshot" LICENSE README.md CHANGELOG.md "$work/$name/"
# The README links the CLI reference, so it ships beside it.
mkdir "$work/$name/docs"
cp docs/usage.md docs/images.md "$work/$name/docs/"
# The built-in font ships inside the binary, so its license ships with it.
cp third_party/jetbrains-mono/OFL.txt "$work/$name/LICENSE-JetBrains-Mono.txt"
tar -czf "dist/$name.tar.gz" -C "$work" "$name"
echo "dist/$name.tar.gz"
