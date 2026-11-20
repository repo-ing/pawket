"""Package a native Pawket build with its launcher and Chrome extension."""

import argparse
from pathlib import Path
from zipfile import ZIP_DEFLATED, ZipFile, ZipInfo

ROOT = Path(__file__).resolve().parents[2]
LAUNCHERS = {
    "linux-x64": "start-pawket.sh",
    "macos-arm64": "Start Pawket.command",
    "macos-x64": "Start Pawket.command",
    "windows-x64": "Start Pawket.cmd",
}


def add_file(archive, source, name, mode=0o644):
    entry = ZipInfo.from_file(source, f"Pawket/{name}")
    entry.create_system = 3
    entry.external_attr = (0o100000 | mode) << 16
    archive.writestr(entry, source.read_bytes(), compress_type=ZIP_DEFLATED)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("platform", choices=LAUNCHERS)
    parser.add_argument("binary", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    windows = args.platform == "windows-x64"
    launcher = "start-pawket.cmd" if windows else "start-pawket.sh"
    with ZipFile(args.output, "w") as archive:
        add_file(archive, args.binary, "pawket.exe" if windows else "pawket", 0o755)
        add_file(archive, ROOT / "scripts/release" / launcher, LAUNCHERS[args.platform], 0o755)
        add_file(archive, ROOT / "scripts/release/START.md", "START.md")
        add_file(archive, ROOT / "LICENSE", "LICENSE")
        for asset in sorted((ROOT / "extension").rglob("*")):
            if asset.is_file():
                add_file(archive, asset, asset.relative_to(ROOT).as_posix())
    print(args.output)


if __name__ == "__main__":
    main()
