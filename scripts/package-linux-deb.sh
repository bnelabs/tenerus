#!/bin/sh
set -eu
umask 022

app_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
bin_path="${1:-$app_root/target/release/tenerus}"
arch="${2:-$(dpkg --print-architecture)}"
out_dir="${3:-$app_root/dist}"
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$app_root/Cargo.toml" | head -n1)

mkdir -p "$out_dir"
staging=$(mktemp -d)
trap 'rm -rf "$staging"' EXIT

pkg="$staging/tenerus_${version}_${arch}"
mkdir -p \
    "$pkg/DEBIAN" \
    "$pkg/usr/bin" \
    "$pkg/usr/share/applications" \
    "$pkg/usr/share/icons/hicolor/256x256/apps" \
    "$pkg/usr/share/doc/tenerus"

install -m 755 "$bin_path" "$pkg/usr/bin/tenerus"
install -m 644 "$app_root/packaging/linux/tenerus.desktop" "$pkg/usr/share/applications/tenerus.desktop"
install -m 644 "$app_root/assets/tenerus-icon-256.png" "$pkg/usr/share/icons/hicolor/256x256/apps/tenerus.png"
install -m 644 "$app_root/LICENSE" "$pkg/usr/share/doc/tenerus/copyright"
install -m 644 "$app_root/README.md" "$pkg/usr/share/doc/tenerus/README.md"
install -m 644 "$app_root/CREDITS.md" "$pkg/usr/share/doc/tenerus/CREDITS.md"
install -m 644 "$app_root/CHANGELOG.md" "$pkg/usr/share/doc/tenerus/CHANGELOG.md"

installed_kb=$(du -sk "$pkg/usr" | awk '{print $1}')

cat > "$pkg/DEBIAN/control" <<EOF
Package: tenerus
Version: ${version}
Section: utils
Priority: optional
Architecture: ${arch}
Installed-Size: ${installed_kb}
Depends: curl, libc6, libgl1 | libglx-mesa0, libx11-6, libxkbcommon0, libwayland-client0
Maintainer: Tenerus Contributors <https://github.com/bnelabs/tenerus>
Homepage: https://github.com/bnelabs/tenerus
Description: Local-first desktop market workspace for educational research
 Tenerus is an independent native desktop market workspace for studying
 historical price distributions, running local analogue replays, and auditing
 AI-assisted market commentary against fixed baselines. Provided solely for
 educational and research purposes.
EOF

dpkg-deb --root-owner-group --build "$pkg" "$out_dir/tenerus_${version}_${arch}.deb"
printf 'Packaged %s\n' "$out_dir/tenerus_${version}_${arch}.deb"
