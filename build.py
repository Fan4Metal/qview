# /// script
# requires-python = ">=3.11"
# dependencies = ["psutil"]
# ///
r"""
Release build: cargo build --release (target\release\qview.exe).

A qview.exe running from this project's target folder locks the file and
makes the link fail, so it is closed first: politely (the window gets
WM_CLOSE and saves its settings), then by force if it has not exited within
3 seconds. Copies of qview elsewhere are left alone.

libheif (tools/build_heif.py) is built the first time and copied next to the
exe, as the release has it.

Run from any folder: uv run build.py [extra cargo arguments]
"""

import subprocess
import sys
from pathlib import Path

import psutil

ROOT = Path(__file__).resolve().parent
TARGET = ROOT / "target"

sys.path.insert(0, str(ROOT / "tools"))
import build_heif  # noqa: E402


def is_ours(proc: psutil.Process) -> bool:
    """Whether `proc` is a qview.exe started from this project's target folder."""
    try:
        exe = Path(proc.exe())
    except (psutil.Error, OSError):
        return False
    return exe.name.lower() == "qview.exe" and TARGET in exe.resolve().parents


def close_running() -> None:
    for proc in psutil.process_iter():
        if not is_ours(proc):
            continue
        print(f"Closing qview.exe (PID {proc.pid})", flush=True)
        # taskkill without /F sends WM_CLOSE, as the window's close button does.
        subprocess.run(["taskkill", "/PID", str(proc.pid)], capture_output=True)
        try:
            proc.wait(timeout=3)
        except psutil.TimeoutExpired:
            proc.kill()
            proc.wait(timeout=5)
        except psutil.NoSuchProcess:
            pass


def main() -> int:
    close_running()
    build_heif.ensure()
    build_heif.copy_to(TARGET / "release")
    return subprocess.run(["cargo", "build", "--release", *sys.argv[1:]], cwd=ROOT).returncode


if __name__ == "__main__":
    sys.exit(main())
