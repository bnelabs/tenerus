#!/bin/sh
set -eu
umask 022

app_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
bin_path="${1:-$app_root/target/release/tenerus}"
arch="${2:-$(uname -m)}"
out_dir="${3:-$app_root/dist}"
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$app_root/Cargo.toml" | head -n1)

mkdir -p "$out_dir"
staging=$(mktemp -d)
trap 'rm -rf "$staging"' EXIT

# 1. Portable FreeBSD tar.xz archive
portable="$staging/tenerus-${version}-freebsd-${arch}"
mkdir -p \
    "$portable/bin" \
    "$portable/share/applications" \
    "$portable/share/icons/hicolor/256x256/apps" \
    "$portable/share/doc/tenerus"

install -m 755 "$bin_path" "$portable/bin/tenerus"
install -m 644 "$app_root/packaging/linux/tenerus.desktop" "$portable/share/applications/tenerus.desktop"
install -m 644 "$app_root/assets/tenerus-icon-256.png" "$portable/share/icons/hicolor/256x256/apps/tenerus.png"
install -m 644 "$app_root/LICENSE" "$portable/share/doc/tenerus/LICENSE"
install -m 644 "$app_root/README.md" "$portable/share/doc/tenerus/README.md"
install -m 644 "$app_root/CREDITS.md" "$portable/share/doc/tenerus/CREDITS.md"
install -m 644 "$app_root/CHANGELOG.md" "$portable/share/doc/tenerus/CHANGELOG.md"

tar -C "$staging" -cJf "$out_dir/tenerus-${version}-freebsd-${arch}.tar.xz" "tenerus-${version}-freebsd-${arch}"
printf 'Packaged %s\n' "$out_dir/tenerus-${version}-freebsd-${arch}.tar.xz"

# 2. Native FreeBSD .pkg package (when FreeBSD pkg(8) is available)
if command -v pkg >/dev/null 2>&1; then
    root="$staging/pkgroot"
    mkdir -p \
        "$root/usr/local/bin" \
        "$root/usr/local/share/applications" \
        "$root/usr/local/share/icons/hicolor/256x256/apps" \
        "$root/usr/local/share/doc/tenerus"

    install -m 755 "$bin_path" "$root/usr/local/bin/tenerus"
    install -m 644 "$app_root/packaging/linux/tenerus.desktop" "$root/usr/local/share/applications/tenerus.desktop"
    install -m 644 "$app_root/assets/tenerus-icon-256.png" "$root/usr/local/share/icons/hicolor/256x256/apps/tenerus.png"
    install -m 644 "$app_root/LICENSE" "$root/usr/local/share/doc/tenerus/LICENSE"
    install -m 644 "$app_root/README.md" "$root/usr/local/share/doc/tenerus/README.md"
    install -m 644 "$app_root/CREDITS.md" "$root/usr/local/share/doc/tenerus/CREDITS.md"
    install -m 644 "$app_root/CHANGELOG.md" "$root/usr/local/share/doc/tenerus/CHANGELOG.md"

    abi=$(pkg config ABI)
    flatsize=$(du -sk "$root" | awk '{print $1 * 1024}')
    curl_ver=$(pkg query '%v' curl 2>/dev/null || true)

    cat > "$staging/+MANIFEST" <<EOF
name: "tenerus"
origin: "finance/tenerus"
version: "${version}"
comment: "Local-first desktop market workspace for educational scenario analysis and AI research"
maintainer: "Tenerus Contributors"
www: "https://github.com/bnelabs/tenerus"
abi: "${abi}"
prefix: "/usr/local"
flatsize: ${flatsize}
licenselogic: "single"
licenses: ["MIT"]
desc: "Tenerus is an independent native desktop market workspace for studying historical price distributions, running local analogue replays, and auditing AI-assisted market commentary against fixed baselines. Provided solely for educational and research purposes."
EOF
    if [ -n "$curl_ver" ]; then
        cat >> "$staging/+MANIFEST" <<EOF
deps: {
  curl: { origin: "ftp/curl", version: "${curl_ver}" }
}
EOF
    fi

    cat > "$staging/plist" <<EOF
bin/tenerus
share/applications/tenerus.desktop
share/icons/hicolor/256x256/apps/tenerus.png
share/doc/tenerus/LICENSE
share/doc/tenerus/README.md
share/doc/tenerus/CREDITS.md
share/doc/tenerus/CHANGELOG.md
EOF

    pkg create -m "$staging" -r "$root" -p "$staging/plist" -o "$out_dir"
    for built in "$out_dir"/tenerus-*.pkg; do
        if [ -f "$built" ] && [ "$built" != "$out_dir/tenerus-${version}-freebsd-${arch}.pkg" ]; then
            mv "$built" "$out_dir/tenerus-${version}-freebsd-${arch}.pkg"
        fi
    done
    printf 'Packaged %s\n' "$out_dir/tenerus-${version}-freebsd-${arch}.pkg"
fi
