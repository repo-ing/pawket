"""Package the Windows release engine and its media tools in a per-user installer."""
import argparse
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
from zipfile import ZipFile

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from media_tools import stage_tools

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("engine_zip", type=Path)
parser.add_argument("output", type=Path)
parser.add_argument("--sha256", default="0d9cdad7d37c66180648fcb391f4c32c29c52ac67a69153a4de48a2f2db2472b")
args = parser.parse_args()
with args.engine_zip.open("rb") as stream:
    if hashlib.file_digest(stream, "sha256").hexdigest() != args.sha256:
        parser.error("Engine ZIP checksum mismatch")
with tempfile.TemporaryDirectory() as temp:
    stage = Path(temp)
    with ZipFile(args.engine_zip) as archive:
        for name in ("pawket.exe", "LICENSE"):
            (stage / name).write_bytes(archive.read("Pawket/" + name))
    stage_tools("windows-x64", stage)
    shutil.copytree(HERE.parent / "licenses", stage / "licenses")
    launcher = (HERE.parent / "start-pawket.cmd").read_text()
    (stage / "Start Pawket.cmd").write_text(launcher, newline="\r\n")
    prefix = "/" if os.name == "nt" else "-"
    subprocess.run(["makensis", prefix + "V2", prefix + "DSTAGE=" + str(stage),
                    prefix + "DOUTPUT=" + str(args.output.resolve()), str(HERE / "setup.nsi")], check=True)
