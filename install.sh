#!/bin/sh
# Install Pawket from its latest GitHub release.
#
#   curl -fsSL https://raw.githubusercontent.com/repo-ing/pawket/main/install.sh | sh
#
# Settings (all optional): PAWKET_REPO (owner/name), PAWKET_INSTALL_DIR,
# PAWKET_BIN_DIR, and PAWKET_ZIP to install a ZIP you already have.
set -eu

repo="${PAWKET_REPO:-repo-ing/pawket}"
install_dir="${PAWKET_INSTALL_DIR:-$HOME/.local/share/pawket}"
bin_dir="${PAWKET_BIN_DIR:-$HOME/.local/bin}"

fail() { printf 'Pawket install: %s\n' "$1" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || fail "this installer needs '$1'. Please install it and try again."; }

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) platform=macos-arm64 ;;
  Darwin-x86_64) platform=macos-x64 ;;
  Linux-x86_64 | Linux-amd64) platform=linux-x64 ;;
  *) fail "there's no ready-made build for $(uname -s) $(uname -m) yet. Build from source: https://github.com/$repo#build-from-source" ;;
esac
need unzip

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM
zip="pawket-$platform.zip"

if [ -n "${PAWKET_ZIP:-}" ]; then
  cp "$PAWKET_ZIP" "$tmp/$zip"
else
  need curl
  base="https://github.com/$repo/releases/latest/download"
  printf 'Downloading %s…\n' "$zip"
  curl -fsSL "$base/$zip" -o "$tmp/$zip" || fail "couldn't download $base/$zip"
  curl -fsSL "$base/SHA256SUMS" -o "$tmp/SHA256SUMS" || fail "couldn't download the checksums"
  expected=$(awk -v name="$zip" '$2 == name { print $1 }' "$tmp/SHA256SUMS")
  if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$tmp/$zip" | awk '{ print $1 }')
  else
    actual=$(shasum -a 256 "$tmp/$zip" | awk '{ print $1 }')
  fi
  [ -n "$expected" ] && [ "$expected" = "$actual" ] || fail "the download didn't match its checksum, so nothing was installed."
fi

unzip -q "$tmp/$zip" -d "$tmp/unpacked"
[ -x "$tmp/unpacked/Pawket/pawket" ] || fail "the ZIP doesn't contain a Pawket program."

# Replace only a previous Pawket install, never an unrelated folder.
if [ -e "$install_dir" ]; then
  [ -x "$install_dir/pawket" ] && [ -f "$install_dir/START.md" ] \
    || fail "$install_dir exists and isn't a Pawket install. Set PAWKET_INSTALL_DIR to another folder."
  rm -rf "$install_dir"
fi
mkdir -p "$(dirname "$install_dir")" "$bin_dir"
mv "$tmp/unpacked/Pawket" "$install_dir"
ln -sf "$install_dir/pawket" "$bin_dir/pawket"

printf '\nPawket is installed: %s\n' "$bin_dir/pawket"
case ":$PATH:" in
  *":$bin_dir:"*) ;;
  *) printf 'Add it to your PATH:  export PATH="%s:$PATH"\n' "$bin_dir" ;;
esac
cat <<'NEXT'

Start it:   pawket serve --open

For YouTube, TikTok, SoundCloud, playlists, MP3, WAV and FLAC, also install
FFmpeg and yt-dlp. Pawket finds them on its own.
  macOS:   brew install ffmpeg yt-dlp deno
  Linux:   your package manager's ffmpeg, plus yt-dlp (pipx install yt-dlp)
NEXT
