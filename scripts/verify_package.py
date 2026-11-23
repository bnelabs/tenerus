"""Verify the local macOS ZIP, allowing only code-signing metadata changes."""
from pathlib import Path, PurePosixPath
import hashlib
import json
import os
import plistlib
import re
import struct
import subprocess
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parent.parent


def sha(data):
    return hashlib.sha256(data).hexdigest()


def normalized_content(data):
    """Exclude the signature blob; normalize only its three size fields.

    Signing changes __LINKEDIT VM/file sizes and LC_CODE_SIGNATURE.datasize.
    Every other byte preceding the signature, including all code/data and UUID,
    must match the linked build. No artifact is modified.
    """
    assert struct.unpack_from('<I', data)[0] == 0xfeedfacf
    output = bytearray(data)
    count = struct.unpack_from('<I', data, 16)[0]
    at = 32
    signature_offset = None
    for _ in range(count):
        command, size = struct.unpack_from('<II', data, at)
        assert size >= 8 and at + size <= len(data)
        if command == 0x19 and data[at + 8:at + 24].rstrip(b'\0') == b'__LINKEDIT':
            struct.pack_into('<Q', output, at + 32, 0)
            struct.pack_into('<Q', output, at + 48, 0)
        elif command == 0x1d:
            signature_offset, signature_size = struct.unpack_from('<II', data, at + 8)
            assert 0 < signature_offset <= signature_offset + signature_size <= len(data)
            struct.pack_into('<I', output, at + 12, 0)
        at += size
    assert signature_offset is not None
    return bytes(output[:signature_offset])


def main():
    archive_path = ROOT / 'Tenerus-macOS.zip'
    app_binary = ROOT / 'Tenerus.app/Contents/MacOS/tenerus'
    build_binary = ROOT / 'target/release/tenerus'
    info_path = 'Contents/Info.plist'
    with tempfile.TemporaryDirectory(prefix='tenerus-package-') as folder:
        destination = Path(folder)
        with zipfile.ZipFile(archive_path) as archive:
            names = archive.namelist()
            assert all(name.startswith('Tenerus.app/') and
                       '..' not in PurePosixPath(name).parts for name in names)
            assert not any('/data/' in name for name in names)
        subprocess.run(['ditto', '-x', '-k', str(archive_path), str(destination)], check=True)
        extracted = destination / 'Tenerus.app'
        subprocess.run(['codesign', '--verify', '--deep', '--strict', str(extracted)], check=True)
        executable = extracted / 'Contents/MacOS/tenerus'
        assert os.access(executable, os.X_OK) and executable.stat().st_mode & 0o777 == 0o755, \
            'The extracted app binary is missing its executable permissions'
        assert (extracted / 'Contents/Resources/CREDITS.md').read_bytes() == (ROOT / 'CREDITS.md').read_bytes()
        assert (extracted / 'Contents/Resources/Tenerus.icns').read_bytes() == (ROOT / 'assets/Tenerus.icns').read_bytes()
        zipped = executable.read_bytes()
        packaged = app_binary.read_bytes()
        built = build_binary.read_bytes()
        assert zipped == packaged, 'ZIP binary differs from the delivered app'
        assert normalized_content(built) == normalized_content(packaged), 'App code/data differs from build'
        info = plistlib.loads((extracted / info_path).read_bytes())
        assert info['CFBundleExecutable'] == executable.name and info['CFBundlePackageType'] == 'APPL'
        assert info['CFBundleIconFile'] == 'Tenerus'
        version = info['CFBundleShortVersionString']
        source_version = re.search(r'^version = "([^"]+)"', (ROOT / 'Cargo.toml').read_text(), re.M)[1]
        assert version == source_version
        result = dict(version=version, signed_binary_sha256=sha(packaged),
                      normalized_content_sha256=sha(normalized_content(packaged)),
                      zip_sha256=sha(archive_path.read_bytes()), archive_files=names,
                      signature_valid=True, zip_binary_matches_app=True,
                      app_code_and_data_match_build=True, icon_verified=True,
                      bundle_contents_verified=True, executable_after_extraction=True,
                      note='Only signature blob and its three size fields are excluded from the build comparison; signed app and ZIP binary match exactly.')
        (ROOT / 'shots').mkdir(exist_ok=True)
        (ROOT / 'shots/package-check.json').write_text(json.dumps(result, indent=2) + '\n')
        print(json.dumps(result))


if __name__ == '__main__':
    main()
