#!/usr/bin/env python3
"""Create deterministic source archives from Git-indexed working-tree files."""
import gzip
import hashlib
from pathlib import Path
import re
import shutil
import subprocess
import tarfile

ROOT = Path(__file__).resolve().parents[1]


def main():
    match = re.search(r"version:\s*'([^']+)'", (ROOT / 'meson.build').read_text())
    if not match or not re.fullmatch(r'[0-9]+\.[0-9]+\.[0-9]+(?:[-.a-zA-Z0-9]*)', match[1]):
        raise SystemExit('Cannot determine a safe release version from meson.build')
    version = match[1]
    paths = subprocess.check_output(['git', 'ls-files', '-z'], cwd=ROOT).decode().split('\0')
    paths = sorted(p for p in paths if p and not p.startswith('packaging/'))
    if 'src/input.c' not in paths or 'LICENSE' not in paths or 'meson_options.txt' not in paths:
        raise SystemExit('Stage the complete source tree before creating a release archive')
    output = ROOT / 'dist' / f'zero-dock-{version}.tar.gz'
    output.parent.mkdir(exist_ok=True)
    with output.open('wb') as raw:
        with gzip.GzipFile(filename='', mode='wb', fileobj=raw, mtime=0) as compressed:
            with tarfile.open(fileobj=compressed, mode='w', format=tarfile.GNU_FORMAT) as archive:
                for relative in paths:
                    source = ROOT / relative
                    if source.is_symlink() or not source.is_file():
                        raise SystemExit(f'Unsupported release entry: {relative}')
                    info = archive.gettarinfo(str(source), arcname=f'zero-dock-{version}/{relative}')
                    info.uid = info.gid = info.mtime = 0
                    info.uname = info.gname = ''
                    info.mode = 0o755 if source.stat().st_mode & 0o111 else 0o644
                    with source.open('rb') as content:
                        archive.addfile(info, content)
    digest = hashlib.sha256(output.read_bytes()).hexdigest()
    (output.parent / 'SHA256SUMS').write_text(f'{digest}  {output.name}\n')
    shutil.copyfile(output, ROOT / 'packaging' / output.name)
    print(f'{digest}  {output}')


if __name__ == '__main__':
    main()
