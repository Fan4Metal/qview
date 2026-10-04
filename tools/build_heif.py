r"""
Builds libheif with libde265 (HEVC) as DLLs for qview into target\heif\bin:
heif.dll and libde265.dll, with the C runtime linked in (/MT), so they need no
Visual C++ Redistributable, and a licenses folder with their licences and where
their sources are. qview loads heif.dll at run time from its own folder
(src\heif.rs); build.py and make_release.py copy these files next to qview.exe,
and the installer and the portable archive take them from there.

Both libraries are LGPL-3.0: they stay separate DLLs that can be replaced.

Needs git and Visual Studio 2022 or later with C++ (the Build Tools' CMake is
used when cmake is not on PATH). The sources are cloned at the tags below into
target\heif\src; a library already built at its tag is not built again.

Run from any folder: python tools/build_heif.py [--clean]
"""

import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
HEIF = ROOT / "target" / "heif"
SRC = HEIF / "src"
BUILD = HEIF / "build"
PREFIX = HEIF / "install"
BIN = HEIF / "bin"

LIBDE265 = ("libde265", "https://github.com/strukturag/libde265", "v1.1.3")
LIBHEIF = ("libheif", "https://github.com/strukturag/libheif", "v1.23.5")

# What goes next to qview.exe, from BIN.
FILES = ["heif.dll", "libde265.dll", "licenses"]

COMMON = [
    "-DBUILD_SHARED_LIBS=ON",
    "-DBUILD_TESTING=OFF",
    # The C runtime linked into each DLL.
    "-DCMAKE_POLICY_DEFAULT_CMP0091=NEW",
    "-DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded",
    f"-DCMAKE_INSTALL_PREFIX={PREFIX}",
    f"-DCMAKE_PREFIX_PATH={PREFIX}",
]

LIBDE265_OPTIONS = ["-DENABLE_SDL=OFF", "-DENABLE_DECODER=OFF", "-DENABLE_ENCODER=OFF"]

# HEVC decoding through libde265 only, built in (no plugins); nothing else.
LIBHEIF_OPTIONS = [
    "-DWITH_LIBDE265=ON",
    "-DWITH_LIBDE265_PLUGIN=OFF",
    "-DENABLE_PLUGIN_LOADING=OFF",
    "-DWITH_X265=OFF",
    "-DWITH_KVAZAAR=OFF",
    "-DWITH_UVG266=OFF",
    "-DWITH_VVDEC=OFF",
    "-DWITH_VVENC=OFF",
    "-DWITH_AOM_DECODER=OFF",
    "-DWITH_AOM_ENCODER=OFF",
    "-DWITH_DAV1D=OFF",
    "-DWITH_RAV1E=OFF",
    "-DWITH_SvtEnc=OFF",
    "-DWITH_JPEG_DECODER=OFF",
    "-DWITH_JPEG_ENCODER=OFF",
    "-DWITH_OpenJPEG_DECODER=OFF",
    "-DWITH_OpenJPEG_ENCODER=OFF",
    "-DWITH_OPENJPH_DECODER=OFF",
    "-DWITH_OPENJPH_ENCODER=OFF",
    "-DWITH_FFMPEG_DECODER=OFF",
    "-DWITH_UNCOMPRESSED_CODEC=OFF",
    "-DWITH_HEADER_COMPRESSION=OFF",
    "-DWITH_LIBSHARPYUV=OFF",
    "-DWITH_EXAMPLES=OFF",
    "-DWITH_GDK_PIXBUF=OFF",
    "-DWITH_GNOME=OFF",
    "-DENABLE_EXPERIMENTAL_FEATURES=OFF",
    "-DCMAKE_DISABLE_FIND_PACKAGE_Doxygen=ON",
]


def cmake() -> str:
    found = shutil.which("cmake")
    if found:
        return found
    for root in (r"C:\Program Files (x86)", r"C:\Program Files"):
        for edition in ("BuildTools", "Community", "Professional", "Enterprise"):
            path = (
                Path(root) / "Microsoft Visual Studio" / "2022" / edition
                / r"Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe"
            )
            if path.exists():
                return str(path)
    sys.exit("CMake not found: install the Visual Studio Build Tools or put cmake on PATH")


def run(*args: str | Path) -> None:
    print(">", " ".join(str(a) for a in args), flush=True)
    subprocess.run([str(a) for a in args], check=True)


def fetch(name: str, url: str, tag: str) -> Path:
    path = SRC / name
    if path.exists():
        current = subprocess.run(
            ["git", "-C", str(path), "describe", "--tags", "--exact-match"], capture_output=True, text=True
        ).stdout.strip()
        if current == tag:
            return path
        shutil.rmtree(path)
    run("git", "-c", "advice.detachedHead=false", "clone", "--depth", "1", "--branch", tag, url, path)
    return path


def build(lib: tuple[str, str, str], options: list[str]) -> None:
    name, url, tag = lib
    stamp = PREFIX / f"{name}.tag"
    if stamp.exists() and stamp.read_text() == tag and (SRC / name / "COPYING").exists():
        print(f"{name} {tag}: built", flush=True)
        return
    source = fetch(name, url, tag)
    out = BUILD / name
    shutil.rmtree(out, ignore_errors=True)
    tool = cmake()
    # CMake's default generator: the newest Visual Studio installed.
    run(tool, "-S", source, "-B", out, "-A", "x64", *COMMON, *options)
    run(tool, "--build", out, "--config", "Release", "--parallel")
    run(tool, "--install", out, "--config", "Release")
    stamp.write_text(tag)


def licenses() -> None:
    folder = BIN / "licenses"
    shutil.rmtree(folder, ignore_errors=True)
    folder.mkdir(parents=True)
    lines = [
        "qview reads HEIC/HEIF images with the libraries below, distributed under",
        "the terms of the GNU Lesser General Public License, version 3 (the texts are",
        "in the files named after each library). They are the separate files",
        "heif.dll and libde265.dll, which can be replaced with other builds of the",
        "same libraries.",
        "",
    ]
    for name, url, tag in (LIBHEIF, LIBDE265):
        shutil.copy2(SRC / name / "COPYING", folder / f"{name}-COPYING.txt")
        lines.append(f"{name} {tag.lstrip('v')}, source: {url}/tree/{tag}")
    lines += ["", "Built by tools/build_heif.py of qview: https://github.com/Fan4Metal/qview", ""]
    (folder / "README.txt").write_text("\r\n".join(lines), encoding="utf-8", newline="")


def ensure() -> Path:
    """Build what is missing; the folder with FILES."""
    PREFIX.mkdir(parents=True, exist_ok=True)
    build(LIBDE265, LIBDE265_OPTIONS)
    build(LIBHEIF, LIBHEIF_OPTIONS)
    BIN.mkdir(parents=True, exist_ok=True)
    for dll in ("heif.dll", "libde265.dll"):
        shutil.copy2(PREFIX / "bin" / dll, BIN / dll)
        print(f"{BIN / dll}: {os.path.getsize(BIN / dll) // 1024} KB", flush=True)
    licenses()
    return BIN


def copy_to(folder: Path) -> None:
    """Put FILES into `folder`, next to qview.exe."""
    folder.mkdir(parents=True, exist_ok=True)
    for name in FILES:
        source, target = BIN / name, folder / name
        if source.is_dir():
            shutil.rmtree(target, ignore_errors=True)
            shutil.copytree(source, target)
        else:
            shutil.copy2(source, target)


def main() -> int:
    if "--clean" in sys.argv[1:]:
        shutil.rmtree(HEIF, ignore_errors=True)
    ensure()
    return 0


if __name__ == "__main__":
    sys.exit(main())
