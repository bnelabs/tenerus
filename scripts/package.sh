#!/bin/sh
set -eu
umask 022
app_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cargo_home="${CARGO_HOME:-$HOME/.cargo}"
sep=$(printf '\037')
CARGO_ENCODED_RUSTFLAGS="--remap-path-prefix=${app_root}=/src${sep}--remap-path-prefix=${cargo_home}=/cargo${sep}--remap-path-prefix=${HOME}=/home" \
    cargo build --manifest-path "$app_root/Cargo.toml" --release --locked
staging=$(mktemp -d /private/tmp/tenerus-sign.XXXXXXXX)
trap 'rm -rf "$staging"' EXIT
bundle="$staging/Tenerus.app"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
cp -X "$app_root/packaging/Info.plist" "$bundle/Contents/Info.plist"
cp -X "$app_root/target/release/tenerus" "$bundle/Contents/MacOS/tenerus"
cp -X "$app_root/assets/Tenerus.icns" "$bundle/Contents/Resources/Tenerus.icns"
cp -X "$app_root/CREDITS.md" "$bundle/Contents/Resources/CREDITS.md"
chmod 755 "$bundle/Contents/MacOS/tenerus"
chmod 644 "$bundle/Contents/Info.plist" "$bundle/Contents/Resources/Tenerus.icns" "$bundle/Contents/Resources/CREDITS.md"
python3 - "$bundle/Contents/Info.plist" "$app_root/Cargo.toml" "$app_root/packaging/Info.plist" <<'PY'
import plistlib, re, sys
from pathlib import Path

info_path, manifest, template_path = map(Path, sys.argv[1:])
version = re.search(r'^version = "([^"]+)"', manifest.read_text(), re.M)[1]
info = plistlib.loads(info_path.read_bytes())
if info['CFBundleShortVersionString'] != version:
    info['CFBundleShortVersionString'] = version
    info['CFBundleVersion'] = str(int(info['CFBundleVersion']) + 1)
    updated = plistlib.dumps(info)
    info_path.write_bytes(updated)
    template_path.write_bytes(updated)
PY
codesign --force --sign - "$bundle"
codesign --verify --deep --strict "$bundle"
python3 - "$staging" "$app_root" <<'PY'
import shutil, sys, zipfile
from pathlib import Path
root = Path(sys.argv[1])
app = Path(sys.argv[2])
with zipfile.ZipFile(root / 'Tenerus-macOS.zip', 'w', compression=zipfile.ZIP_DEFLATED, compresslevel=6) as archive:
    for path in sorted((root / 'Tenerus.app').rglob('*')):
        if path.is_file():
            archive.write(path, path.relative_to(root))
current = app / 'Tenerus.app'
backup = root / 'previous-bundle'
had_current = current.exists()
if had_current:
    current.rename(backup)
try:
    (root / 'Tenerus.app').rename(current)
except OSError:
    if had_current:
        backup.rename(current)
    raise
try:
    (root / 'Tenerus-macOS.zip').replace(app / 'Tenerus-macOS.zip')
except OSError:
    shutil.rmtree(current, ignore_errors=True)
    if had_current:
        backup.rename(current)
    raise
PY
# Check the workspace copy's signature and strictly verify the clean ZIP copy below.
codesign --verify --deep "$app_root/Tenerus.app"
python3 "$app_root/scripts/verify_package.py"
printf 'Packaged Tenerus.app\n'
