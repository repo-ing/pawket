# Start Pawket

Extract the entire ZIP, then open the Pawket folder.

- **macOS:** double-click **Start Pawket.command**.
- **Windows:** double-click **Start Pawket.cmd**.
- **Linux:** run **./start-pawket.sh** from a terminal.

Pawket opens in your browser at <http://127.0.0.1:3987/>. Paste a link, press
**Find media**, pick Video or Audio and the quality, then **Save to Pawket**. Click **Open folder** to find the file. Use **Choose folder** to change where future saves go. Keep the
terminal window open while you use it, and press Ctrl+C there to stop it.

## More sites and formats

On its own, Pawket saves X posts, direct file links and media on ordinary web
pages. Install FFmpeg and yt-dlp to add YouTube, TikTok, SoundCloud, Vimeo,
playlists and more, plus MP3, WAV, FLAC, Opus and "plays everywhere" video.
Pawket finds them automatically when they're on your PATH.

- **macOS:** `brew install ffmpeg yt-dlp deno`
- **Windows:** `winget install Gyan.FFmpeg yt-dlp.yt-dlp DenoLand.Deno`
- **Linux:** FFmpeg from your package manager, and `pipx install yt-dlp`

Deno lets yt-dlp handle YouTube's player checks. See `pawket serve --help` for
all options, including `--cookies-from-browser` for videos that need your login.

## Chrome extension

1. Start Pawket and open the local page.
2. In Chrome, open `chrome://extensions`, turn on Developer mode, then choose
   **Load unpacked** and select the bundled `extension` folder.
3. On the Pawket page, click **Ready** and copy its pairing code.
4. Open the extension's **Connection** panel, paste the code, and save it.
   Pair again when Pawket restarts.

These builds are not code-signed yet. macOS and Windows may ask you to approve
running the launcher or program the first time. Only run a build downloaded
from the official GitHub releases.
