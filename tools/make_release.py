"""
Release build: libheif (build_heif.py, built the first time), cargo build
--release, the Inno Setup installer (dist\\qview_<version>_Setup.exe) and the
portable archive (dist\\qview_<version>_portable.zip), both with heif.dll,
libde265.dll and their licences next to the exe.

Runs from any folder: python tools/make_release.py [--no-tests] [--install]
--install then installs the build silently over the installed copy, to try it.
No external dependencies. The output of cargo and ISCC is shown as is, so that
the progress of the build is visible.
"""

import argparse
import os
import re
import shutil
import subprocess
import sys
import time
import winreg
import zipfile
from pathlib import Path

import build_heif

ROOT = Path(__file__).resolve().parent.parent
CARGO_TOML = ROOT / "Cargo.toml"
ISS_FILE = ROOT / "tools" / "setup.iss"
DIST_DIR = ROOT / "dist"
EXE = ROOT / "target" / "release" / "qview.exe"
ICON = ROOT / "target" / "app.ico"

# Files the installer takes from the repository (see [Files] in setup.iss).
BUNDLED_FILES = ["LICENSE", "README.md", "README.ru.md"]
# Folder inside the portable archive, so that extracting "here" does not drop the exe among other files.
PORTABLE_DIR = "qview"

# The installer's AppId (setup.iss) and where Inno Setup records the installation.
APP_ID = "{A06A0881-69DE-4D38-86C4-EC1FC1D9D86C}"
UNINSTALL_KEY = Rf"Software\Microsoft\Windows\CurrentVersion\Uninstall\{APP_ID}_is1"
# Silent installation: no wizard, no questions, no restart; the previous choices
# (folder, tasks such as the file type registration) are used again.
SILENT_INSTALL = ["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/CLOSEAPPLICATIONS"]

ISCC_PATHS = [
    Path(R"C:\Program Files (x86)\Inno Setup 6\ISCC.exe"),
    Path(R"C:\Program Files\Inno Setup 6\ISCC.exe"),
    Path(os.environ.get("LOCALAPPDATA", "")) / "Programs" / "Inno Setup 6" / "ISCC.exe",
]


class ReleaseError(Exception):
    """Build failure with a message ready to be shown."""


def human_size(num_bytes: int) -> str:
    num = float(num_bytes)
    for unit in ("B", "KB", "MB", "GB"):
        if num < 1024:
            return f"{num:.1f} {unit}" if unit != "B" else f"{int(num)} {unit}"
        num /= 1024
    return f"{num:.1f} TB"


def fmt_cmd(command: list[str]) -> str:
    return " ".join(f'"{c}"' if " " in c else c for c in command)


def run_command(command: list[str], title: str) -> None:
    """Run a command, showing its output as is; stop the build if it fails."""
    print(f"$ {fmt_cmd(command)}\n")
    started = time.monotonic()
    result = subprocess.run(command, cwd=ROOT)
    elapsed = time.monotonic() - started
    if result.returncode != 0:
        raise ReleaseError(f"{title}: the command exited with code {result.returncode} (after {elapsed:.0f} s)")
    print(f"\n{title}: done in {elapsed:.0f} s")


class Steps:
    """Print step headers and the time the previous step took."""

    def __init__(self, total: int):
        self.total = total
        self.number = 0
        self.started: float | None = None

    def _close(self) -> None:
        if self.started is not None:
            print(f"--- step took {time.monotonic() - self.started:.1f} s")

    def next(self, title: str) -> None:
        self._close()
        self.number += 1
        print(f"\n=== [{self.number}/{self.total}] {title} ===")
        self.started = time.monotonic()

    def finish(self) -> None:
        self._close()
        self.started = None


def extract_version(path: Path) -> str:
    """Version from the [package] section of Cargo.toml."""
    content = path.read_text(encoding="utf-8")
    package = re.search(r"^\[package\](.*?)(?=^\[|\Z)", content, re.S | re.M)
    match = package and re.search(r'^version\s*=\s*"([^"]+)"', package.group(1), re.M)
    if not match:
        raise ReleaseError(f"package version not found in {path}")
    return match.group(1)


def windows_version(version: str) -> str:
    """0.1.0 -> 0.1.0.0: VersionInfoVersion needs four numbers."""
    numbers = [int(n) for n in re.findall(r"\d+", version.split("-")[0])][:4]
    return ".".join(str(n) for n in numbers + [0] * (4 - len(numbers)))


def find_cargo() -> str:
    found = shutil.which("cargo")
    if found:
        return found
    # rustup installed with --no-modify-path leaves cargo off PATH in new shells.
    fallback = Path.home() / ".cargo" / "bin" / "cargo.exe"
    if fallback.is_file():
        return str(fallback)
    raise ReleaseError("cargo not found: install Rust (rustup) or add ~/.cargo/bin to PATH")


def find_iscc() -> Path:
    for path in ISCC_PATHS:
        if path.is_file():
            return path
    found = shutil.which("ISCC")
    if found:
        return Path(found)
    raise ReleaseError("ISCC.exe not found: install Inno Setup 6 or add ISCC to PATH")


def exe_locked(exe: Path) -> bool:
    if not exe.is_file():
        return False
    try:
        with exe.open("r+b"):
            return False
    except PermissionError:
        return True


def close_running_exe(exe: Path) -> None:
    """A running exe is locked by Windows, and cargo or the installer could not overwrite it.

    The copy started from `exe` is closed as build.py does it: WM_CLOSE first (taskkill without /F),
    so that it saves its settings, then by force after 3 seconds."""
    if not exe_locked(exe):
        return
    query = f"(Get-Process qview -ErrorAction SilentlyContinue | Where-Object Path -eq '{exe}').Id"
    out = subprocess.run(["powershell", "-NoProfile", "-Command", query], capture_output=True, text=True)
    pids = out.stdout.split()
    for pid in pids:
        print(f"  closing qview.exe (PID {pid})")
        subprocess.run(["taskkill", "/PID", pid], capture_output=True)
    deadline = time.monotonic() + 3
    while exe_locked(exe) and time.monotonic() < deadline:
        time.sleep(0.2)
    if exe_locked(exe):
        for pid in pids:
            subprocess.run(["taskkill", "/F", "/PID", pid], capture_output=True)
        time.sleep(0.5)
    if exe_locked(exe):
        raise ReleaseError(f"{exe} is in use: close qview and try again")


def installed_exe() -> Path:
    """The exe of the installed copy: where Inno Setup recorded it, or the default folder."""
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, UNINSTALL_KEY) as key:
            folder = Path(winreg.QueryValueEx(key, "InstallLocation")[0])
    except OSError:
        folder = Path(os.environ.get("LOCALAPPDATA", "")) / "Programs" / "qview"
    return folder / EXE.name


def check_prerequisites() -> tuple[str, Path]:
    missing = [name for name in BUNDLED_FILES if not (ROOT / name).is_file()]
    if missing:
        raise ReleaseError("files for the installer not found: " + ", ".join(missing))
    cargo = find_cargo()
    iscc = find_iscc()
    print(f"  cargo:                 {cargo}")
    print(f"  Inno Setup:            {iscc}")
    return cargo, iscc


def make_portable_zip(version: str) -> Path:
    """Archive with the exe, libheif and the licenses in the PORTABLE_DIR folder (the documentation is on GitHub)."""
    archive = DIST_DIR / f"qview_{version}_portable.zip"
    with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        z.write(EXE, f"{PORTABLE_DIR}/{EXE.name}")
        z.write(ROOT / "LICENSE", f"{PORTABLE_DIR}/LICENSE")
        for name in build_heif.FILES:
            path = EXE.parent / name
            for file in sorted(path.rglob("*")) if path.is_dir() else [path]:
                if file.is_file():
                    z.write(file, f"{PORTABLE_DIR}/{file.relative_to(EXE.parent).as_posix()}")
    return archive


def main() -> int:
    parser = argparse.ArgumentParser(description="Build a qview release")
    parser.add_argument("--no-tests", action="store_true", help="do not run cargo test")
    parser.add_argument(
        "--install", action="store_true", help="then install the build silently over the installed copy"
    )
    args = parser.parse_args()

    # Line buffering: with output redirected to a file, step headers still come before the tools' output.
    sys.stdout.reconfigure(line_buffering=True)
    total_started = time.monotonic()
    steps = Steps((5 if args.no_tests else 6) + args.install)
    try:
        steps.next("Checks")
        version = extract_version(CARGO_TOML)
        print(f"  version:               {version} (from {CARGO_TOML.name})")
        cargo, iscc = check_prerequisites()

        steps.next("libheif")
        # Before the tests, which decode HEIC through it.
        build_heif.ensure()

        if not args.no_tests:
            steps.next("Tests")
            run_command([cargo, "test"], "cargo test")

        steps.next("Release build")
        # Checked right before the build: qview may have been started while the tests ran.
        close_running_exe(EXE)
        run_command([cargo, "build", "--release"], "cargo build")
        if not EXE.is_file():
            raise ReleaseError(f"cargo finished, but {EXE} was not found")
        build_heif.copy_to(EXE.parent)

        steps.next("Installer icon")
        ICON.unlink(missing_ok=True)
        run_command([str(EXE), "--export-icon", str(ICON)], "icon export")
        if not ICON.is_file():
            raise ReleaseError(f"icon not created: {ICON}")

        steps.next("Inno Setup installer")
        run_command(
            [
                str(iscc),
                f"/DMyAppVersion={version}",
                f"/DVersionInfoVersion={windows_version(version)}",
                f"/DAppIcon={ICON}",
                str(ISS_FILE),
            ],
            "ISCC",
        )
        installer = DIST_DIR / f"qview_{version}_Setup.exe"
        if not installer.is_file():
            raise ReleaseError(f"ISCC finished, but the installer was not found: {installer}")
        # Names without spaces: GitHub replaces spaces in release file names with dots.
        portable = make_portable_zip(version)
        print(f"Portable version: {portable.relative_to(ROOT)}")

        if args.install:
            steps.next("Silent installation")
            installed = installed_exe()
            # Closed here rather than by the installer (/CLOSEAPPLICATIONS is the fallback), so that
            # it saves its settings.
            close_running_exe(installed)
            run_command([str(installer), *SILENT_INSTALL], "installation")
            if not installed.is_file():
                raise ReleaseError(f"the installer finished, but {installed} was not found")
            print(f"Installed: {installed}")
        steps.finish()

    except ReleaseError as e:
        print(f"\nERROR: build stopped: {e}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        print("\nERROR: build interrupted by the user", file=sys.stderr)
        return 1

    minutes, seconds = divmod(int(time.monotonic() - total_started), 60)
    print(f"\n=== Release {version} built in {minutes} min {seconds} s ===")
    print(f"  installer:   {installer}  ({human_size(installer.stat().st_size)})")
    print(f"  portable:    {portable}  ({human_size(portable.stat().st_size)})")
    if args.install:
        print(f"  installed:   {installed}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
