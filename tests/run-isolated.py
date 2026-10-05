#!/usr/bin/env python3
"""Run the Rust GUI host on a private X server and D-Bus session."""
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
    parser.add_argument("build", nargs="?", default="build", help="Meson build directory")
    parser.add_argument("--iterations", type=int, default=3, help="Number of host lifecycle runs")
    parser.add_argument("--seconds", type=int, default=3, help="Seconds per lifecycle run")
    parser.add_argument("--private-audio", action="store_true", help="Use disposable PipeWire/Pulse servers")
    parser.add_argument("--scale", type=int, choices=(1, 2), default=1)
    parser.add_argument("--geometry", default="1024x768")
    parser.add_argument("--inside", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()

    if not 1 <= args.iterations <= 100:
        parser.error("Iterations must be between 1 and 100")
    if not 1 <= args.seconds <= 300:
        parser.error("Seconds must be between 1 and 300")
    if "x" not in args.geometry:
        parser.error("Geometry must be WIDTHxHEIGHT")

    build = (ROOT / args.build).resolve()
    executable = build / "zero-dock-test-host"
    if not executable.is_file():
        parser.error(f"Test host missing: {executable}; configure with -Dtests=true")

    required = ("dbus-run-session", "Xvfb", "xfwm4")
    for command in required:
        if shutil.which(command) is None:
            parser.error(f"Required test command missing: {command}")

    if not args.inside:
        with tempfile.TemporaryDirectory(
            prefix="zero-dock-session-", ignore_cleanup_errors=True
        ) as temporary:
            env = os.environ.copy()
            env.update(
                XDG_CONFIG_HOME=temporary,
                XDG_CACHE_HOME=temporary,
                NO_AT_BRIDGE="1",
                GIO_USE_VFS="local",
                GDK_SCALE=str(args.scale),
            )
            env.pop("DBUS_SESSION_BUS_ADDRESS", None)
            command = [
                "dbus-run-session",
                "--",
                sys.executable,
                __file__,
                str(build),
                "--inside",
                "--iterations",
                str(args.iterations),
                "--seconds",
                str(args.seconds),
                "--scale",
                str(args.scale),
                "--geometry",
                args.geometry,
            ]
            if args.private_audio:
                command.append("--private-audio")
            return subprocess.run(command, env=env, check=False).returncode

    read_fd, write_fd = os.pipe()
    xvfb = wm = pipewire = pulse = policy = None
    try:
        xvfb = subprocess.Popen(
            [
                "Xvfb",
                "-displayfd",
                str(write_fd),
                "-screen",
                "0",
                args.geometry + "x24",
                "-nolisten",
                "tcp",
                "+extension",
                "Composite",
            ],
            pass_fds=(write_fd,),
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        os.close(write_fd)
        write_fd = None
        if not select.select([read_fd], [], [], 10)[0]:
            raise RuntimeError("Xvfb did not allocate a display within 10 seconds")
        display = os.read(read_fd, 32).decode().strip()
        if not display.isdigit():
            raise RuntimeError("Xvfb display allocation failed")

        env = os.environ.copy()
        env.update(DISPLAY=":" + display)
        env.pop("XAUTHORITY", None)
        env.pop("SESSION_MANAGER", None)

        if args.private_audio:
            for binary in ("pipewire", "pipewire-pulse", "wireplumber", "pactl"):
                if shutil.which(binary) is None:
                    raise RuntimeError(f"Private audio requires {binary}")
            runtime = Path(os.environ["XDG_CONFIG_HOME"]) / "runtime"
            runtime.mkdir(mode=0o700)
            env.update(
                XDG_RUNTIME_DIR=str(runtime),
                PIPEWIRE_RUNTIME_DIR=str(runtime),
                PIPEWIRE_REMOTE="pipewire-0",
                PULSE_SERVER="unix:" + str(runtime / "pulse/native"),
                PIPEWIRE_PULSE_RUNTIME_DIR=str(runtime / "pulse"),
            )
            pipewire = subprocess.Popen(
                ["pipewire"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
            )
            policy = subprocess.Popen(
                ["wireplumber", "--profile", "policy"],
                env=env,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            pulse = subprocess.Popen(
                ["pipewire-pulse"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
            )
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline:
                result = subprocess.run(
                    ["pactl", "info"],
                    env=env,
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                    timeout=2,
                    check=False,
                )
                if result.returncode == 0:
                    break
                time.sleep(0.1)
            else:
                raise RuntimeError("Private Pulse server did not start")

        wm = subprocess.Popen(
            ["xfwm4", "--compositor=on", "--vblank=off"],
            env=env,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        time.sleep(1)
        if wm.poll() is not None:
            raise RuntimeError("The isolated window manager failed to start")

        rc_path = Path(os.environ["XDG_CONFIG_HOME"]) / "zero-dock-smoke.rc"
        host_env = env.copy()
        host_env["G_DEBUG"] = "fatal-warnings"
        for iteration in range(args.iterations):
            result = subprocess.run(
                [str(executable), str(rc_path), str(args.seconds)],
                env=host_env,
                timeout=args.seconds + 15,
                check=False,
            )
            if result.returncode != 0:
                print(
                    f"zero-dock-test-host failed on lifecycle {iteration + 1}/{args.iterations}: "
                    f"exit {result.returncode}",
                    file=sys.stderr,
                )
                return result.returncode
        return 0
    finally:
        stop(pulse)
        stop(policy)
        stop(pipewire)
        if write_fd is not None:
            os.close(write_fd)
        os.close(read_fd)
        stop(wm)
        stop(xvfb)


if __name__ == "__main__":
    sys.exit(main())
