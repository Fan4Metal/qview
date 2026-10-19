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


def post_close(pid: int) -> None:
    """WM_CLOSE posted to the top-level windows of `pid`, as the window's close button does, so that
    it saves its settings: those with a title but the system's (the input method's, the OpenGL
    driver's, winit's message target); the visible ones, else (a window hidden in the tray) the
    hidden ones. taskkill without /F skipped a minimized window, which then could only be killed."""
    import ctypes
    from ctypes import wintypes

    service = ("IME", "MSCTFIME UI", "NVOpenGLPbuffer", "Winit Thread Event Target")
    user32 = ctypes.windll.user32
    visible, hidden = [], []

    @ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    def each(hwnd, _):
        owner = wintypes.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        name = ctypes.create_unicode_buffer(64)
        user32.GetClassNameW(hwnd, name, 64)
        if owner.value == pid and user32.GetWindowTextLengthW(hwnd) > 0 and name.value not in service:
            (visible if user32.IsWindowVisible(hwnd) else hidden).append(hwnd)
        return True

    user32.EnumWindows(each, 0)
    for hwnd in visible or hidden:
        user32.PostMessageW(hwnd, 0x0010, 0, 0)  # WM_CLOSE


def close_running() -> None:
    for proc in psutil.process_iter():
        if not is_ours(proc):
            continue
        print(f"Closing qview.exe (PID {proc.pid})", flush=True)
        post_close(proc.pid)
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
