#!/usr/bin/env python3
"""Run tests on a private X server and D-Bus; leave the desktop untouched."""
import argparse
import os
from pathlib import Path
import select
import signal
import re
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
    parser.add_argument('--private-audio', action='store_true', help='Use disposable PipeWire/Pulse servers')
    parser.add_argument('--real-apps', action='store_true', help='Test installed Chrome and Thunar on the isolated desktop')
    parser.add_argument('--soak-seconds', type=int, default=0, help='Run the window lifecycle stress path for this duration')
    parser.add_argument('--scale', type=int, choices=(1, 2), default=1)
    parser.add_argument('--geometry', default='1024x768')
    parser.add_argument('--inside', action='store_true', help=argparse.SUPPRESS)
    args = parser.parse_args()
    if not re.fullmatch(r'[1-9][0-9]{2,3}x[1-9][0-9]{2,3}', args.geometry):
        parser.error('Geometry must be WIDTHxHEIGHT, each between 100 and 9999')
    if not 0 <= args.soak_seconds <= 86400:
        parser.error('Stress duration must be between 0 and 86400 seconds')
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
                       NO_AT_BRIDGE='1', GIO_USE_VFS='local', ZERO_DOCK_ISOLATED_TEST='1',
                       GDK_SCALE=str(args.scale), ZERO_DOCK_REAL_APPS=str(int(args.real_apps)),
                       ZERO_DOCK_SOAK_SECONDS=str(args.soak_seconds))
            env.pop('DBUS_SESSION_BUS_ADDRESS', None)
            command = ['dbus-run-session', '--', sys.executable, __file__, str(build), '--inside']
            command.extend(['--geometry', args.geometry, '--soak-seconds', str(args.soak_seconds)])
            if args.private_audio:
                command.append('--private-audio')
            if args.filter:
                command.extend(['--filter', args.filter])
            return subprocess.run(command, env=env, check=False).returncode

    read_fd, write_fd = os.pipe()
    wm = xvfb = pipewire = pulse = policy = test_process = None
    try:
        xvfb = subprocess.Popen(
            ['Xvfb', '-displayfd', str(write_fd), '-screen', '0', args.geometry + 'x24',
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
        env.pop('SESSION_MANAGER', None)
        if args.private_audio:
            for binary in ('pipewire', 'pipewire-pulse', 'wireplumber', 'pactl'):
                if not shutil.which(binary):
                    raise RuntimeError(f'Private audio requires {binary}')
            runtime = Path(os.environ['XDG_CONFIG_HOME']) / 'runtime'
            runtime.mkdir(mode=0o700)
            env.update(XDG_RUNTIME_DIR=str(runtime), PIPEWIRE_RUNTIME_DIR=str(runtime),
                       PIPEWIRE_REMOTE='pipewire-0', PULSE_SERVER='unix:' + str(runtime / 'pulse/native'),
                       PIPEWIRE_PULSE_RUNTIME_DIR=str(runtime / 'pulse'))
            pipewire = subprocess.Popen(['pipewire'], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            policy = subprocess.Popen(['wireplumber', '--profile', 'policy'], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            pulse = subprocess.Popen(['pipewire-pulse'], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline:
                result = subprocess.run(['pactl', 'info'], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=2)
                if result.returncode == 0:
                    break
                time.sleep(.1)
            else:
                raise RuntimeError('Private Pulse server did not start')
            subprocess.run(['pactl', 'load-module', 'module-null-sink', 'sink_name=zero-dock-test'], env=env, check=True, stdout=subprocess.DEVNULL)
            time.sleep(.5)
            subprocess.run(['pactl', 'set-default-sink', 'zero-dock-test'], env=env, check=True)
            env['ZERO_DOCK_PRIVATE_PULSE_PID'] = str(pulse.pid)
        wm = subprocess.Popen(['xfwm4', '--compositor=on', '--vblank=off'],
                              env=env, stdout=subprocess.DEVNULL)
        time.sleep(1)
        if wm.poll() is not None:
            raise RuntimeError('The isolated window manager failed to start')
        command = [str(executable)]
        test_filter = args.filter or env.get('ZERO_DOCK_TEST_FILTER')
        if test_filter:
            command.extend(['-p', test_filter])
        if env.get('ZERO_DOCK_TEST_DEBUG') == '1':
            command = ['gdb', '--batch', '--return-child-result', '-ex', 'run', '-ex', 'thread apply all bt', '--args', *command]
        test_process = subprocess.Popen(command, env=env, start_new_session=True)
        return test_process.wait(timeout=max(90, args.soak_seconds + 60))
    finally:
        if test_process:
            try:
                os.killpg(test_process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            stop(test_process)
            time.sleep(.1)
            try:
                os.killpg(test_process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        stop(pulse)
        stop(policy)
        stop(pipewire)
        os.close(read_fd)
        if write_fd is not None:
            os.close(write_fd)
        stop(wm)
        stop(xvfb)


if __name__ == '__main__':
    sys.exit(main())
