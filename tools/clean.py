#!/usr/bin/env python3
"""Remove this checkout's generated files; keep tracked files and releases."""
import argparse
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def candidates():
    for path in ROOT.iterdir():
        if path.name.startswith('build') and (path / 'meson-private').is_dir():
            yield path
    for name in ('evidence', 'stage', 'packaging/src', 'packaging/pkg'):
        path = ROOT / name
        if path.exists():
            yield path
    for path in (ROOT / 'packaging').glob('*'):
        if path.name.endswith(('.tar.gz', '.pkg.tar.zst')):
            yield path
    for directory in ('src', 'tests', 'tools'):
        yield from (ROOT / directory).rglob('__pycache__')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--apply', action='store_true', help='Delete listed artifacts')
    args = parser.parse_args()
    tracked = subprocess.check_output(['git', 'ls-files', '-z'], cwd=ROOT).decode().split('\0')
    for path in sorted(set(candidates())):
        relative = path.relative_to(ROOT).as_posix()
        if path.is_symlink():
            raise SystemExit(f'Refusing generated-directory symlink: {relative}')
        if any(p == relative or p.startswith(relative + '/') for p in tracked if p):
            raise SystemExit(f'Refusing to remove tracked content: {relative}')
        print(('Remove ' if args.apply else 'Would remove ') + relative)
        if args.apply:
            if path.is_dir():
                shutil.rmtree(path)
            else:
                path.unlink()


if __name__ == '__main__':
    main()
