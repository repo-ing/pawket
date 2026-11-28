# Link to finished MP3

On an Apple silicon Mac mini, Pawket v0.3.5 saved all 11 YouTube videos and Shorts in a median of **2.83 seconds**. The yt-dlp CLI took **8.65 seconds**. Both produced 128 kbps MP3s on all 11 links.

| Public YouTube link | Audio length | Pawket | yt-dlp |
| --- | ---: | ---: | ---: |
| [xXpsMebk-Mk](https://www.youtube.com/watch?v=xXpsMebk-Mk) | 5:29 | 2.83 s | 8.65 s |
| [jNQXAC9IVRw](https://www.youtube.com/watch?v=jNQXAC9IVRw) | 0:19 | 1.38 s | 2.56 s |
| [aqz-KE-bpKQ](https://www.youtube.com/watch?v=aqz-KE-bpKQ) | 10:35 | 3.22 s | 8.81 s |
| [M7lc1UVf-VE](https://www.youtube.com/watch?v=M7lc1UVf-VE) | 22:24 | 5.42 s | 13.72 s |
| [xJy1bCLx_Kc](https://www.youtube.com/watch?v=xJy1bCLx_Kc) | 1:47 | 1.77 s | 3.12 s |
| [dQw4w9WgXcQ](https://www.youtube.com/watch?v=dQw4w9WgXcQ) | 3:33 | 7.31 s | 10.09 s |
| [LXb3EKWsInQ](https://www.youtube.com/watch?v=LXb3EKWsInQ) | 5:14 | 7.25 s | 10.84 s |
| [9bZkp7q19f0](https://www.youtube.com/watch?v=9bZkp7q19f0) | 4:12 | 7.24 s | 9.24 s |
| [18NGQq7p3LY](https://www.youtube.com/shorts/18NGQq7p3LY) | 0:03 | 1.28 s | 2.44 s |
| [BGQWPY4IigY](https://www.youtube.com/shorts/BGQWPY4IigY) | 0:14 | 1.57 s | 3.85 s |
| [TlIB_iKhEkk](https://www.youtube.com/shorts/TlIB_iKhEkk) | 0:13 | 1.33 s | 2.50 s |

## How we measured

The clock started at job submission in an open Pawket app or at CLI launch. It stopped when the MP3 was ready on disk. Each tool saved each link to a fresh output folder. We checked codec, bitrate and duration with ffprobe after timing. Pawket uses yt-dlp for YouTube extraction and FFmpeg for conversion.

We tested the same links in separate runs on macOS 26.5.1. Network conditions and site availability can change between runs. The CLI comparison used yt-dlp 2026.11.23 with the following options.

```sh
yt-dlp --ignore-config --no-playlist -f bestaudio/best -x --audio-format mp3 --audio-quality 128K -o "out.%(ext)s" "VIDEO_URL"
```

## Beyond YouTube

Both tools saved 18 of 20 links across YouTube, Instagram, Vimeo and direct files. On the Instagram Reel that both saved, yt-dlp took 1.97 seconds and Pawket took 3.56 seconds. Both failed on the other Reel and the Vimeo link. The table above uses the full Pawket rerun after a local storage error on one link in the first pass.
