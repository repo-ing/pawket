"""Stage the checksum-pinned upstream executables for one release platform."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tarfile
from zipfile import ZipFile


def stage_tools(platform, directory):
    manifest = Path(__file__).with_name("media-tools.json")
    assets = json.loads(manifest.read_text())[platform]
    staged = []
    for name, asset in assets.items():
        download = directory / asset["url"].rsplit("/", 1)[1]
        subprocess.run(["curl", "--fail", "--silent", "--show-error", "--location",
                        "--retry", "3", "--output", str(download), asset["url"]], check=True)
        with download.open("rb") as source:
            if hashlib.file_digest(source, "sha256").hexdigest() != asset["sha256"]:
                raise ValueError(f"Checksum mismatch for {name} on {platform}")
        filename = name + (".exe" if platform.startswith("windows") else "")
        tool = directory / "tools" / filename
        tool.parent.mkdir(exist_ok=True)
        entry = asset.get("entry")
        if not entry:
            download.rename(tool)
        elif download.suffix == ".zip":
            with ZipFile(download) as archive, archive.open(entry) as source, tool.open("wb") as dest:
                shutil.copyfileobj(source, dest)
        else:
            with tarfile.open(download) as archive, archive.extractfile(entry) as source, tool.open("wb") as dest:
                shutil.copyfileobj(source, dest)
        tool.chmod(0o755)
        staged.append(tool)
    return staged
