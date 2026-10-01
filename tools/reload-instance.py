#!/usr/bin/env python3
"""Reload one external Zero Dock wrapper using XFCE's restart signal."""
import argparse
from datetime import datetime
from pathlib import Path
import re
import shutil
import os
import signal
import subprocess
import time


def run(*args):
    return subprocess.check_output(args, text=True).strip()


def properties():
    return dict(line.split(None, 1) for line in
                run('xfconf-query', '-c', 'xfce4-panel', '-lv').splitlines())


def wrappers(identifier):
    result = []
    for path in Path('/proc').iterdir():
        if not path.name.isdigit():
            continue
        try:
            args = (path / 'cmdline').read_bytes().split(b'\0')
        except (FileNotFoundError, PermissionError, ProcessLookupError):
            continue
        if (len(args) > 3 and Path(args[0].decode(errors='replace')).name == 'wrapper-2.0'
                and Path(args[1].decode(errors='replace')).name == 'libzero-dock.so'
                and args[2] == str(identifier).encode()):
            result.append(int(path.name))
    return result


def wait_for(check, description):
    deadline = time.monotonic() + 8
    while time.monotonic() < deadline:
        result = check()
        if result:
            return result
        time.sleep(.1)
    raise RuntimeError(f'Timed out waiting for {description}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--instance', type=int, required=True)
    parser.add_argument('--apply', action='store_true', help='Reload the instance')
    args = parser.parse_args()
    current = properties()
    plugin = f'/plugins/plugin-{args.instance}'
    if current.get(plugin) != 'zero-dock':
        raise SystemExit('The selected instance is not Zero Dock')
    panels = {}
    for name, value in current.items():
        if re.fullmatch(r'/panels/panel-\d+/plugin-ids', name):
            ids = [int(i) for i in value.strip('[]').split(',') if i.strip()]
            if args.instance in ids:
                panels[name] = ids
    if len(panels) != 1:
        raise SystemExit('Expected one panel containing the selected instance')
    name, ids = next(iter(panels.items()))
    print(f'Zero Dock instance {args.instance}: {name}, preserving order {ids}')
    if not args.apply:
        return
    run('gdbus', 'call', '--session', '--dest', 'org.xfce.Panel',
        '--object-path', '/org/xfce/Panel', '--method', 'org.xfce.Panel.Save')
    config = Path.home() / '.config/xfce4/panel' / f'zero-dock-{args.instance}.rc'
    if not config.is_file():
        raise SystemExit('Configuration file missing; instance was left running')
    backup = (Path.home() / '.local/state/zero-dock/backups' /
              datetime.now().astimezone().strftime('%Y%m%d-%H%M%S'))
    backup.mkdir(parents=True, mode=0o700)
    snapshot = backup / config.name
    shutil.copy2(config, snapshot)
    panel_config = Path.home() / '.config/xfce4/xfconf/xfce-perchannel-xml/xfce4-panel.xml'
    if panel_config.is_file():
        shutil.copy2(panel_config, backup / panel_config.name)
    old = wrappers(args.instance)
    if len(old) != 1:
        raise SystemExit('Expected one running external wrapper for the selected instance')
    # XFCE 4.20 treats a wrapper's SIGUSR1 exit as a requested restart:
    # https://gitlab.xfce.org/xfce/xfce4-panel/-/blob/xfce4-panel-4.20.8/panel/panel-plugin-external.c#L744
    # Save and back up first; leave both panel item lists and plugin IDs intact.
    os.kill(old[0], signal.SIGUSR1)
    pids = wait_for(lambda: [pid for pid in wrappers(args.instance) if pid not in old],
                    'the replacement dock wrapper')
    time.sleep(.5)
    if not wrappers(args.instance):
        raise RuntimeError('Replacement wrapper exited during startup')
    if properties().get(name) != current[name]:
        raise RuntimeError('Panel item order changed unexpectedly')
    print(f'Reloaded instance {args.instance}; wrapper PID {pids[0]}; backup {snapshot}')


if __name__ == '__main__':
    main()
