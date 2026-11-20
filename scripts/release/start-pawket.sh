#!/bin/sh
set -eu
app_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
# Opened from Finder, the shell may not have Homebrew's folders on PATH;
# Pawket looks there for FFmpeg and yt-dlp.
PATH="$PATH:/opt/homebrew/bin:/usr/local/bin"
export PATH
case "$(uname -s)" in
    Darwin) data_dir="$HOME/Library/Application Support/Pawket" ;;
    *) data_dir="${XDG_DATA_HOME:-$HOME/.local/share}/pawket" ;;
esac
exec "$app_dir/pawket" serve --open --output "$data_dir"
