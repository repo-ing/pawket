"""Build the standalone Apple silicon app from the public v0.3.0 engine ZIP."""
import hashlib
import plistlib
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from zipfile import ZipFile

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from media_tools import stage_tools

def run(*args):
    subprocess.run(args, check=True)

def main(engine_zip, output, expected, version):
    with engine_zip.open("rb") as stream:
        if hashlib.file_digest(stream, "sha256").hexdigest() != expected:
            raise SystemExit("Engine ZIP checksum mismatch")
    with tempfile.TemporaryDirectory() as tmp:
        work = Path(tmp)
        app = work / "Pawket.app"
        contents = app / "Contents"
        helpers, macos, resources = [contents / folder for folder in ("Helpers", "MacOS", "Resources")]
        for folder in (helpers, macos, resources):
            folder.mkdir(parents=True)
        with ZipFile(engine_zip) as archive:
            (helpers / "pawket").write_bytes(archive.read("Pawket/pawket"))
            (resources / "LICENSE").write_bytes(archive.read("Pawket/LICENSE"))
        (helpers / "pawket").chmod(0o755)
        for tool in stage_tools("macos-arm64", work):
            if tool.is_dir():
                shutil.copytree(tool, resources / tool.name, symlinks=True)
            elif tool.is_symlink():
                (helpers / tool.name).symlink_to(Path("../Resources") / tool.readlink())
            else:
                shutil.copy2(tool, helpers / tool.name, follow_symlinks=False)
        shutil.copytree(HERE.parent / "licenses", resources / "licenses")
        shutil.copy2(HERE.parent / "media-tools.json", resources / "upstream.json")
        run("swiftc", "-O", "-target", "arm64-apple-macos14.0", HERE / "main.swift", "-o", macos / "Pawket")
        icons = work / "Pawket.iconset"
        icons.mkdir()
        for size in (16, 32, 128, 256, 512):
            for scale in (1, 2):
                pixels = str(size * scale)
                suffix = "@2x" if scale == 2 else ""
                run("sips", "-z", pixels, pixels, HERE.parents[2] / "assets/brand/pawket-logo.png",
                    "--out", icons / f"icon_{size}x{size}{suffix}.png")
        run("iconutil", "-c", "icns", icons, "-o", resources / "Pawket.icns")
        (contents / "Info.plist").write_bytes(plistlib.dumps(dict(
            CFBundleName="Pawket", CFBundleDisplayName="Pawket", CFBundleExecutable="Pawket",
            CFBundleIdentifier="app.repoing.pawket.launcher", CFBundlePackageType="APPL",
            CFBundleIconFile="Pawket", CFBundleShortVersionString=version, CFBundleVersion="1",
            LSMinimumSystemVersion="14.0", NSHighResolutionCapable=True)))
        run("codesign", "--force", "--sign", "-", app)
        run("codesign", "--verify", "--deep", "--strict", app)
        run("ditto", "-c", "-k", "--keepParent", app, output)

if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("engine_zip", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--sha256", default="102382db45d26ede2633bac4de4b565532d1aed266f45f533a433bec9b226339")
    parser.add_argument("--version", default="0.3.0")
    args = parser.parse_args()
    main(args.engine_zip, args.output.resolve(), args.sha256, args.version)
