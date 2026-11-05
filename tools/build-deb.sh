#!/bin/bash
# Builds a .deb of a tagged release for the system it runs on, from a clean
# export of the tag (never the working tree), with the tag's Cargo.lock.
#
#     tools/build-deb.sh v0.5.0 [OUTPUT_DIR]     # default: target/deb
#
# Needs dpkg-dev (dpkg-deb, dpkg-shlibdeps) and the build packages in README.md.
# Set CARGO_TARGET_DIR to share compiled crates between several builds.
set -euo pipefail
umask 022

tag=${1:?usage: build-deb.sh TAG [OUTPUT_DIR]}
repo=$(git rev-parse --show-toplevel)
out=$(realpath -m "${2:-$repo/target/deb}")

version=${tag#v}
revision=1
arch=$(dpkg --print-architecture)
deb=fedigoo_$version-${revision}_$arch.deb
app_id=com.aisloperator.FediGoo

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
src=$work/src
root=$work/root
mkdir -p "$src" "$root/DEBIAN" "$out"
git -C "$repo" archive "$tag" | tar -x -C "$src"

toml_version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$src/Cargo.toml" | head -n 1)
if [ "$toml_version" != "$version" ]; then
    echo "$tag: Cargo.toml says version $toml_version" >&2
    exit 1
fi

# Keep this machine's paths out of the binary (panic messages include them).
cargo_home=${CARGO_HOME:-$HOME/.cargo}
export RUSTFLAGS="--remap-path-prefix=$src=/fedigoo --remap-path-prefix=$cargo_home=/cargo"
export SOURCE_DATE_EPOCH=$(git -C "$repo" log -1 --format=%ct "$tag^{commit}")
target=${CARGO_TARGET_DIR:-$src/target}
(cd "$src" && CARGO_TARGET_DIR=$target cargo build --release --locked)

install -D -m 755 "$target/release/fedigoo" "$root/usr/bin/fedigoo"
mkdir -p "$root/usr/share/applications"
sed 's/@EXEC@/fedigoo/' "$src/data/$app_id.desktop.in" \
    > "$root/usr/share/applications/$app_id.desktop"
chmod 644 "$root/usr/share/applications/$app_id.desktop"
install -D -m 644 "$src/data/icons/$app_id.svg" \
    "$root/usr/share/icons/hicolor/scalable/apps/$app_id.svg"

doc=$root/usr/share/doc/fedigoo
mkdir -p "$doc"
cat > "$doc/copyright" <<EOF
FediGoo
Created by AI Sloperator with Claude Code.
https://www.aisloperator.com/
Source: https://github.com/aisloperator/fedigoo

No license is given.
EOF
chmod 644 "$doc/copyright"
cat <<EOF | gzip -9n > "$doc/changelog.Debian.gz"
fedigoo ($version-$revision) trixie; urgency=medium

  * Release $version:
    https://github.com/aisloperator/fedigoo/releases/tag/$tag

 -- AI Sloperator <me@aisloperator.com>  $(date -u -R -d "@$SOURCE_DATE_EPOCH")
EOF
chmod 644 "$doc/changelog.Debian.gz"

# dpkg-shlibdeps insists on running where it can see a debian/control.
mkdir "$work/debian"
touch "$work/debian/control"
shlib_depends=$(cd "$work" && dpkg-shlibdeps -O "$root/usr/bin/fedigoo" 2>/dev/null \
    | sed -n 's/^shlibs:Depends=//p')

# GTK needs librsvg's loader for the SVG app icon; TLS and the icon themes
# come in through the dependencies of libsoup and GTK.
cat > "$root/DEBIAN/control" <<EOF
Package: fedigoo
Version: $version-$revision
Section: net
Priority: optional
Architecture: $arch
Installed-Size: $(du -sk --exclude=DEBIAN "$root" | cut -f1)
Depends: $shlib_depends
Recommends: librsvg2-common
Maintainer: AI Sloperator <me@aisloperator.com>
Homepage: https://github.com/aisloperator/fedigoo
Description: Mastodon client for the Linux desktop
 FediGoo is a Mastodon client for the Linux desktop, written in Rust with
 GTK 4.
EOF

dpkg-deb --root-owner-group -Zxz --build "$root" "$out/$deb" > /dev/null
echo "$out/$deb"
