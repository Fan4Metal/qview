r"""
Builds libheif with libde265 (HEVC) and dav1d (AV1) as DLLs for qview into
target\heif\bin: heif.dll, with dav1d linked into it, and libde265.dll, with the
C runtime linked in (/MT), so they need no Visual C++ Redistributable. qview
loads heif.dll at run time from its own folder (src\heif.rs) for HEIC, HEIF and
AVIF; build.py and make_release.py copy these files next to qview.exe, and the
installer and the portable archive take them from there.

libheif and libde265 are LGPL-3.0: they stay separate DLLs that can be
replaced. dav1d is BSD-2-Clause, so it may be linked into heif.dll. Their
licences (licenses\LGPL-3.0.txt, licenses\dav1d-license.txt) and the source of
these tags (THIRD_PARTY_LICENSES.md, which a test checks against LIBHEIF,
LIBDE265 and DAV1D) are in the repository and shipped from there.

Needs git and Visual Studio 2022 or later with C++ (the Build Tools' CMake is
used when cmake is not on PATH). dav1d is built with Meson and Ninja, which pip
puts into target\heif\tools, and its assembly with NASM, downloaded there (its
archive checked against NASM_SHA256). The sources are cloned at the tags below
into target\heif\src; a library already built at its tag with the same options
is not built again.

Run from any folder: python tools/build_heif.py [--clean]
"""

import hashlib
import os
import shutil
import subprocess
import sys
import urllib.request
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
HEIF = ROOT / "target" / "heif"
SRC = HEIF / "src"
BUILD = HEIF / "build"
PREFIX = HEIF / "install"
BIN = HEIF / "bin"
TOOLS = HEIF / "tools"

LIBDE265 = ("libde265", "https://github.com/strukturag/libde265", "v1.1.3")
DAV1D = ("dav1d", "https://code.videolan.org/videolan/dav1d.git", "1.5.4")
LIBHEIF = ("libheif", "https://github.com/strukturag/libheif", "v1.23.5")

# NASM for dav1d's assembly: without it dav1d decodes several times slower.
# nasm.us publishes no checksums; this is the hash of the archive first used.
NASM_VERSION = "3.01"
NASM_URL = f"https://www.nasm.us/pub/nasm/releasebuilds/{NASM_VERSION}/win64/nasm-{NASM_VERSION}-win64.zip"
NASM_SHA256 = "e0ba5157007abc7b1a65118a96657a961ddf55f7e3f632ee035366dfce039ca4"
MESON = "meson==1.12.1"
NINJA = "ninja==1.13.2"

# What goes next to qview.exe, from BIN.
FILES = ["heif.dll", "libde265.dll"]

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

# A static library with the C runtime linked in (b_vscrt=mt), 8 and 10/12 bits.
DAV1D_OPTIONS = [
    "--buildtype=release",
    "--default-library=static",
    "-Db_vscrt=mt",
    "-Denable_tools=false",
    "-Denable_tests=false",
    "-Denable_examples=false",
    "-Denable_docs=false",
    "-Dbitdepths=8,16",
    "-Denable_asm=true",
]

# HEVC decoding through libde265 and AV1 decoding through dav1d, both built in
# (no plugins); nothing else. Meson names dav1d's static library libdav1d.a,
# which libheif's FindDAV1D does not look for under MSVC, so it is named.
LIBHEIF_OPTIONS = [
    "-DWITH_LIBDE265=ON",
    "-DWITH_LIBDE265_PLUGIN=OFF",
    "-DWITH_DAV1D=ON",
    "-DWITH_DAV1D_PLUGIN=OFF",
    f"-DDAV1D_INCLUDE_DIR={PREFIX / 'include'}",
    f"-DDAV1D_LIBRARY={PREFIX / 'lib' / 'libdav1d.a'}",
    "-DENABLE_PLUGIN_LOADING=OFF",
    "-DWITH_X265=OFF",
    "-DWITH_KVAZAAR=OFF",
    "-DWITH_UVG266=OFF",
    "-DWITH_VVDEC=OFF",
    "-DWITH_VVENC=OFF",
    "-DWITH_AOM_DECODER=OFF",
    "-DWITH_AOM_ENCODER=OFF",
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


def run(*args: str | Path, env: dict[str, str] | None = None) -> None:
    print(">", " ".join(str(a) for a in args), flush=True)
    subprocess.run([str(a) for a in args], check=True, env=env)


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


def stamp_text(tag: str, options: list[str]) -> str:
    """What a library's stamp holds: its tag and its options, so that a new option builds it again."""
    return tag + "\n" + "\n".join(options)


def built(name: str, tag: str, options: list[str]) -> bool:
    stamp = PREFIX / f"{name}.tag"
    if stamp.exists() and stamp.read_text() == stamp_text(tag, options) and (SRC / name / "COPYING").exists():
        print(f"{name} {tag}: built", flush=True)
        return True
    return False


def build(lib: tuple[str, str, str], options: list[str]) -> None:
    name, url, tag = lib
    if built(name, tag, options):
        return
    source = fetch(name, url, tag)
    out = BUILD / name
    shutil.rmtree(out, ignore_errors=True)
    tool = cmake()
    # CMake's default generator: the newest Visual Studio installed.
    run(tool, "-S", source, "-B", out, "-A", "x64", *COMMON, *options)
    run(tool, "--build", out, "--config", "Release", "--parallel")
    run(tool, "--install", out, "--config", "Release")
    (PREFIX / f"{name}.tag").write_text(stamp_text(tag, options))


def nasm() -> Path:
    """The folder with nasm.exe, downloaded the first time."""
    folder = TOOLS / f"nasm-{NASM_VERSION}"
    if (folder / "nasm.exe").exists():
        return folder
    TOOLS.mkdir(parents=True, exist_ok=True)
    archive = TOOLS / f"nasm-{NASM_VERSION}-win64.zip"
    print(f"> download {NASM_URL}", flush=True)
    with urllib.request.urlopen(NASM_URL, timeout=60) as response:
        data = response.read()
    digest = hashlib.sha256(data).hexdigest()
    if digest != NASM_SHA256:
        sys.exit(f"{NASM_URL}: SHA-256 {digest}, expected {NASM_SHA256}")
    archive.write_bytes(data)
    with zipfile.ZipFile(archive) as z:
        z.extractall(TOOLS)
    archive.unlink()
    return folder


def python_tools() -> tuple[Path, Path]:
    """Meson and Ninja installed by pip into TOOLS: the folder for PYTHONPATH and the one with ninja.exe."""
    site = TOOLS / "python"
    marker = site / "installed.txt"
    wanted = f"{MESON} {NINJA}"
    if not (marker.exists() and marker.read_text() == wanted):
        shutil.rmtree(site, ignore_errors=True)
        run(sys.executable, "-m", "pip", "install", "--disable-pip-version-check", "--target", site, MESON, NINJA)
        marker.write_text(wanted)
    return site, site / "bin"


def build_dav1d() -> None:
    name, url, tag = DAV1D
    if built(name, tag, DAV1D_OPTIONS):
        return
    source = fetch(name, url, tag)
    site, ninja_dir = python_tools()
    env = dict(os.environ)
    env["PATH"] = os.pathsep.join([str(nasm()), str(ninja_dir), env.get("PATH", "")])
    env["PYTHONPATH"] = str(site)
    meson = [sys.executable, "-m", "mesonbuild.mesonmain"]
    out = BUILD / name
    shutil.rmtree(out, ignore_errors=True)
    # --vsenv: Meson sets up the newest Visual Studio's environment itself.
    run(*meson, "setup", "--vsenv", f"--prefix={PREFIX}", "--libdir=lib", *DAV1D_OPTIONS, out, source, env=env)
    run(*meson, "compile", "-C", out, env=env)
    run(*meson, "install", "-C", out, "--no-rebuild", env=env)
    (PREFIX / f"{name}.tag").write_text(stamp_text(tag, DAV1D_OPTIONS))


def ensure() -> Path:
    """Build what is missing; the folder with FILES."""
    PREFIX.mkdir(parents=True, exist_ok=True)
    build(LIBDE265, LIBDE265_OPTIONS)
    build_dav1d()
    build(LIBHEIF, LIBHEIF_OPTIONS)
    BIN.mkdir(parents=True, exist_ok=True)
    for dll in FILES:
        shutil.copy2(PREFIX / "bin" / dll, BIN / dll)
        print(f"{BIN / dll}: {os.path.getsize(BIN / dll) // 1024} KB", flush=True)
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
