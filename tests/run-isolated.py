#!/usr/bin/env python3
"""Run tests on a private X server and D-Bus; leave the desktop untouched."""
import argparse
import os
from pathlib import Path
import select
import shutil
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]


def stop(process):
    if process is not None and process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('build', nargs='?', default='build', help='Meson build directory')
    parser.add_argument('--filter', help='Run one GLib test path')
    parser.add_argument('--inside', action='store_true', help=argparse.SUPPRESS)
    args = parser.parse_args()
    build = (ROOT / args.build).resolve()
    executable = build / 'zero-dock-integration'
    if not executable.is_file():
        parser.error(f'Test executable missing: {executable}; configure with -Dtests=true')
    for command in ('dbus-run-session', 'Xvfb', 'xfwm4', 'pacat'):
        if shutil.which(command) is None:
            parser.error(f'Required test command missing: {command}')
    if not args.inside:
        with tempfile.TemporaryDirectory(prefix='zero-dock-session-') as temporary:
            env = os.environ.copy()
            env.update(XDG_CONFIG_HOME=temporary, XDG_CACHE_HOME=temporary,
                       NO_AT_BRIDGE='1', GIO_USE_VFS='local', ZERO_DOCK_ISOLATED_TEST='1')
            env.pop('DBUS_SESSION_BUS_ADDRESS', None)
            command = ['dbus-run-session', '--', sys.executable, __file__, str(build), '--inside']
            if args.filter:
                command.extend(['--filter', args.filter])
            return subprocess.run(command, env=env, check=False).returncode

    read_fd, write_fd = os.pipe()
    wm = xvfb = None
    try:
        xvfb = subprocess.Popen(
            ['Xvfb', '-displayfd', str(write_fd), '-screen', '0', '1024x768x24',
             '-nolisten', 'tcp', '+extension', 'Composite'],
            pass_fds=(write_fd,), stdout=subprocess.DEVNULL)
        os.close(write_fd)
        write_fd = None
        if not select.select([read_fd], [], [], 10)[0]:
            raise RuntimeError('Xvfb did not allocate a display within 10 seconds')
        display = os.read(read_fd, 32).decode().strip()
        if not display.isdigit():
            raise RuntimeError('Xvfb display allocation failed')
        env = os.environ.copy()
        env.update(DISPLAY=':' + display, ZERO_DOCK_TEST_BUILD=str(build))
        env.pop('XAUTHORITY', None)
        wm = subprocess.Popen(['xfwm4', '--compositor=on', '--vblank=off'],
                              env=env, stdout=subprocess.DEVNULL)
        time.sleep(1)
        if wm.poll() is not None:
            raise RuntimeError('The isolated window manager failed to start')
        command = [str(executable)]
        test_filter = args.filter or env.get('ZERO_DOCK_TEST_FILTER')
        if test_filter:
            command.extend(['-p', test_filter])
        return subprocess.run(command, env=env, timeout=45, check=False).returncode
    finally:
        os.close(read_fd)
        if write_fd is not None:
            os.close(write_fd)
        stop(wm)
        stop(xvfb)


if __name__ == '__main__':
    sys.exit(main())
