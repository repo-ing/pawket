"""Pawket's private, pipe-driven extractor for public YouTube audio tracks."""
import json
import re
import sys

from yt_dlp.extractor.common import InfoExtractor


class PawketAudioIE(InfoExtractor):
    _VALID_URL = r"pawket-audio:worker$"

    def _real_extract(self, url):
        engine = self._downloader

        def reply(message):
            print(json.dumps(message), file=sys.stdout, flush=True)

        def progress(update):
            if update.get("status") == "downloading":
                reply({"written": update.get("downloaded_bytes", 0),
                       "total": update.get("total_bytes")})

        engine.add_progress_hook(progress)
        for line in sys.stdin:
            request = json.loads(line)
            video_id = request["id"]
            if not re.fullmatch(r"[a-zA-Z0-9_-]{11}", video_id):
                reply({"done": False})
                continue
            engine.params["paths"] = {"home": request["directory"]}
            try:
                info = engine.extract_info("https://www.youtube.com/watch?v=" + video_id,
                                           ie_key="Youtube", download=True)
                reply({"done": bool(info and info.get("requested_downloads"))})
            except Exception:
                reply({"done": False})
        return self.playlist_result([])
