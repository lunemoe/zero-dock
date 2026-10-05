#!/usr/bin/env python3
"""Run Rust integration scenarios in an isolated X11/XFCE session."""
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

CORE = (
    "native-all",
    "pinned-lifecycle",
    "launch-feedback",
    "identity-workspaces",
    "improvements",
    "preview-pixels",
)


def stop(process):
    if process is not None and process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


def wait_pulse(env):
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
            return
        time.sleep(0.1)
    raise RuntimeError("Private Pulse server did not start")


def pactl_retry(args, env, timeout=5.0):
    """Run a pactl command, retrying while the private audio server warms up."""
    deadline = time.monotonic() + timeout
    while True:
        result = subprocess.run(
            args, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False
        )
        if result.returncode == 0:
            return
        if time.monotonic() >= deadline:
            raise RuntimeError(f"Failed to run: {' '.join(args)}")
        time.sleep(0.1)


def configure_private_audio(env):
    for binary in ("pipewire", "pipewire-pulse", "wireplumber", "pactl", "pacat"):
        if shutil.which(binary) is None:
            raise RuntimeError(f"Private audio requires {binary}")

    runtime = Path(os.environ["XDG_CONFIG_HOME"]) / "runtime"
    runtime.mkdir(mode=0o700, exist_ok=True)
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
    wait_pulse(env)

    pactl_retry(["pactl", "load-module", "module-null-sink", "sink_name=zero-dock-test"], env)
    pactl_retry(["pactl", "set-default-sink", "zero-dock-test"], env)
    env["ZERO_DOCK_PRIVATE_PULSE_PID"] = str(pulse.pid)
    return pipewire, policy, pulse


def selected_scenarios(args):
    if args.scenario:
        return tuple(args.scenario)
    if args.suite == "core":
        return CORE
    if args.suite == "stress":
        return ("stress",)
    if args.suite == "audio":
        return ("audio-recovery",)
    if args.suite == "real-apps":
        return ("real-apps",)
    if args.suite == "all":
        return CORE + ("stress", "audio-recovery", "real-apps")
    raise AssertionError(args.suite)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("build", nargs="?", default="build", help="Meson build directory")
    parser.add_argument(
        "--suite",
        choices=("core", "stress", "audio", "real-apps", "all"),
        default="core",
    )
    parser.add_argument("--scenario", action="append", help="Run an explicit scenario (repeatable)")
    parser.add_argument("--private-audio", action="store_true")
    parser.add_argument("--scale", type=int, choices=(1, 2), default=1)
    parser.add_argument("--geometry", default="1280x900")
    parser.add_argument("--stress-cycles", type=int, default=5)
    parser.add_argument("--inside", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()

    if not 1 <= args.stress_cycles <= 100:
        parser.error("stress cycles must be between 1 and 100")
    if "x" not in args.geometry:
        parser.error("Geometry must be WIDTHxHEIGHT")

    build = (ROOT / args.build).resolve()
    executable = build / "zero-dock-integration"
    if not executable.is_file():
        parser.error(f"Integration binary missing: {executable}; configure with -Dtests=true")

    required = ("dbus-run-session", "Xvfb", "xfwm4")
    for command in required:
        if shutil.which(command) is None:
            parser.error(f"Required test command missing: {command}")

    scenarios = selected_scenarios(args)
    needs_audio = args.private_audio or "audio-recovery" in scenarios

    if not args.inside:
        with tempfile.TemporaryDirectory(
            prefix="zero-dock-integration-", ignore_cleanup_errors=True
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
                "--suite",
                args.suite,
                "--scale",
                str(args.scale),
                "--geometry",
                args.geometry,
                "--stress-cycles",
                str(args.stress_cycles),
            ]
            for scenario in args.scenario or ():
                command.extend(["--scenario", scenario])
            if needs_audio:
                command.append("--private-audio")
            return subprocess.run(command, env=env, check=False).returncode

    read_fd, write_fd = os.pipe()
    xvfb = wm = pipewire = policy = pulse = None
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
                "+extension",
                "XTEST",
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
        env.update(
            DISPLAY=":" + display,
            ZERO_DOCK_TEST_BUILD=str(build),
            ZERO_DOCK_STRESS_CYCLES=str(args.stress_cycles),
        )
        env.pop("XAUTHORITY", None)
        env.pop("SESSION_MANAGER", None)
        env.pop("G_DEBUG", None)

        if needs_audio:
            pipewire, policy, pulse = configure_private_audio(env)

        wm = subprocess.Popen(
            ["xfwm4", "--compositor=on", "--vblank=off"],
            env=env,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        time.sleep(1)
        if wm.poll() is not None:
            raise RuntimeError("The isolated window manager failed to start")

        # Make workspace behavior deterministic when xfconf-query is available.
        if shutil.which("xfconf-query"):
            subprocess.run(
                [
                    "xfconf-query",
                    "-c",
                    "xfwm4",
                    "-p",
                    "/general/workspace_count",
                    "-n",
                    "-t",
                    "int",
                    "-s",
                    "2",
                ],
                env=env,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                check=False,
            )
            subprocess.run(
                [
                    "xfconf-query",
                    "-c",
                    "xfwm4",
                    "-p",
                    "/general/workspace_count",
                    "-s",
                    "2",
                ],
                env=env,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                check=False,
            )
            time.sleep(0.2)

        test_env = env.copy()
        test_env["G_DEBUG"] = "fatal-warnings"
        for scenario in scenarios:
            print(f"--- isolated scenario: {scenario} ---", flush=True)
            timeout = 90 if scenario == "stress" else 45
            if scenario == "audio-recovery":
                timeout = 30
            result = subprocess.run(
                [str(executable), "--scenario", scenario],
                env=test_env,
                timeout=timeout,
                check=False,
            )
            if result.returncode != 0:
                print(
                    f"integration scenario {scenario} failed: exit {result.returncode}",
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
