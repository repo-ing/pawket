//! Encode frame-aligned blocks of PCM. Overlap primes the analysis filter at
//! each boundary. Disabling the bit reservoir makes the retained frames independent.
use super::*;
use futures_util::{stream::FuturesOrdered, StreamExt};
use std::io::SeekFrom;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

const SAMPLES: usize = 1152;
const BLOCK: usize = 1536;
const OVERLAP: usize = 12;
const INVALID: PawketError = PawketError::ConversionFailed;

#[derive(Clone, Copy)]
struct Pcm {
    rate: usize,
    channels: usize,
}

impl Pcm {
    fn from_header(h: &[u8; 44]) -> Result<Self> {
        let rate = u32::from_le_bytes(h[24..28].try_into().unwrap()) as usize;
        let channels = u16::from_le_bytes([h[22], h[23]]) as usize;
        if &h[..4] != b"RIFF"
            || &h[8..16] != b"WAVEfmt "
            || h[16..22] != [16, 0, 0, 0, 1, 0]
            || h[34..36] != [16, 0]
            || &h[36..40] != b"data"
            || !matches!(rate, 32000 | 44100 | 48000)
            || !matches!(channels, 1 | 2)
        {
            return Err(INVALID);
        }
        Ok(Self { rate, channels })
    }

    fn frame_bytes(self) -> usize {
        SAMPLES * self.channels * 2
    }
}

struct Encoded {
    header: Vec<u8>,
    frames: u32,
    audio: Vec<u8>,
}

async fn encode(
    executable: &Path,
    directory: &Path,
    pcm: Pcm,
    data: Vec<u8>,
    skip: usize,
    last: bool,
) -> Result<Encoded> {
    let (output, path) = storage::create_temporary(directory).await?;
    let mut child = Command::new(executable)
        .args([
            "-v",
            "error",
            "-nostdin",
            "-xerror",
            "-protocol_whitelist",
            "pipe",
            "-f",
            "s16le",
            "-ar",
        ])
        .arg(pcm.rate.to_string())
        .arg("-ac")
        .arg(pcm.channels.to_string())
        .args([
            "-i",
            "pipe:0",
            "-map_metadata",
            "-1",
            "-id3v2_version",
            "0",
            "-c:a",
            "libmp3lame",
            "-reservoir",
            "0",
            "-compression_level",
            "7",
            "-b:a",
            "128k",
            "-f",
            "mp3",
            "-fd",
            "1",
            "fd:",
        ])
        .stdin(Stdio::piped())
        .stdout(output.into_std().await)
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let mut stdin = child.stdin.take().ok_or(INVALID)?;
    stdin.write_all(&data).await?;
    drop(stdin);
    drop(data);
    if !child.wait().await?.success() {
        return Err(INVALID);
    }
    let bytes = tokio::fs::read(&path).await?;
    let mut pos = 0;
    let mut index = 0;
    let mut header = Vec::new();
    let mut audio = Vec::new();
    let mut frames = 0;
    while pos < bytes.len() {
        let h = bytes.get(pos..pos + 4).ok_or(INVALID)?;
        if h[0] != 0xff || h[1] != 0xfb || h[2] >> 4 != 9 {
            return Err(INVALID);
        }
        let size = 144 * 128000 / pcm.rate + usize::from((h[2] >> 1) & 1);
        let frame = bytes.get(pos..pos + size).ok_or(INVALID)?;
        if index == 0 {
            header.extend_from_slice(frame);
        } else if index > skip && (last || index <= skip + BLOCK) {
            audio.extend_from_slice(frame);
            frames += 1;
        }
        pos += size;
        index += 1;
    }
    if frames == 0 || (!last && frames != BLOCK as u32) {
        return Err(INVALID);
    }
    Ok(Encoded {
        header,
        frames,
        audio,
    })
}

fn crc_table() -> [u16; 256] {
    std::array::from_fn(|i| {
        let mut v = i as u16;
        for _ in 0..8 {
            v = if v & 1 == 1 {
                (v >> 1) ^ 0xa001
            } else {
                v >> 1
            };
        }
        v
    })
}

fn crc(data: &[u8], mut value: u16, table: &[u16; 256]) -> u16 {
    for &b in data {
        value = (value >> 8) ^ table[usize::from((value as u8) ^ b)];
    }
    value
}

pub(super) async fn run(
    executable: &Path,
    input: &Path,
    directory: &Path,
) -> Result<(TempPath, u64)> {
    let workers = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(4);
    if workers < 2 {
        return Err(INVALID);
    }
    let input = File::open(input).await?.into_std().await;
    let mut decoder = Command::new(executable)
        .args([
            "-v",
            "error",
            "-nostdin",
            "-xerror",
            "-protocol_whitelist",
            "fd",
            "-format_whitelist",
            "mov,matroska,webm,ogg,mp3,aac,wav,flac",
            "-threads",
            "2",
            "-max_pixels",
            "67108864",
            "-fd",
            "0",
            "-i",
            "fd:",
            "-map",
            "0:a:0",
            "-map_metadata",
            "-1",
            "-af",
            "aformat=sample_fmts=s16:channel_layouts=mono|stereo",
            "-c:a",
            "pcm_s16le",
            "-fflags",
            "+bitexact",
            "-f",
            "wav",
            "pipe:1",
        ])
        .stdin(input)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let mut reader = decoder.stdout.take().ok_or(INVALID)?;
    let mut wav = [0; 44];
    reader.read_exact(&mut wav).await?;
    let pcm = Pcm::from_header(&wav)?;
    let stride = pcm.frame_bytes();
    let mut prefix = Vec::new();
    let mut ahead = Vec::new();
    let mut pending = FuturesOrdered::new();
    let (mut out, path) = storage::create_temporary(directory).await?;
    let mut frames = 0u32;
    let mut bytes = 0u32;
    let table = crc_table();
    let mut music_crc = 0;
    let mut final_header = Vec::new();
    let mut ended = false;
    while !ended || !pending.is_empty() {
        while !ended && pending.len() < workers {
            let skip = prefix.len() / stride;
            let mut data = std::mem::take(&mut prefix);
            data.append(&mut ahead);
            let wanted = (skip + BLOCK + OVERLAP) * stride;
            (&mut reader)
                .take((wanted - data.len()) as u64)
                .read_to_end(&mut data)
                .await?;
            ended = data.len() < wanted;
            if !ended {
                let cut = (skip + BLOCK) * stride;
                prefix = data[cut - OVERLAP * stride..cut].to_vec();
                ahead = data[cut..].to_vec();
            }
            pending.push_back(encode(executable, directory, pcm, data, skip, ended));
        }
        let part = pending.next().await.ok_or(INVALID)??;
        if bytes == 0 {
            out.write_all(&part.header).await?;
            bytes = part.header.len() as u32;
        }
        bytes = bytes.checked_add(part.audio.len() as u32).ok_or(INVALID)?;
        frames = frames.checked_add(part.frames).ok_or(INVALID)?;
        music_crc = crc(&part.audio, music_crc, &table);
        out.write_all(&part.audio).await?;
        final_header = part.header;
    }
    if !decoder.wait().await?.success() {
        return Err(INVALID);
    }
    // Keep the final block's encoder delay and end padding. All preceding
    // blocks end on MPEG frame boundaries. CBR permits a linear seek table.
    let k = final_header
        .windows(4)
        .position(|w| w == b"Info")
        .ok_or(INVALID)?;
    if final_header.len() < 190
        || final_header.len() < k + 156
        || final_header[k + 4..k + 8] != 15u32.to_be_bytes()
    {
        return Err(INVALID);
    }
    final_header[k + 8..k + 12].copy_from_slice(&frames.to_be_bytes());
    final_header[k + 12..k + 16].copy_from_slice(&bytes.to_be_bytes());
    for (i, v) in final_header[k + 16..k + 116].iter_mut().enumerate() {
        *v = (i * 256 / 100) as u8;
    }
    final_header[k + 148..k + 152].copy_from_slice(&bytes.to_be_bytes());
    final_header[k + 152..k + 154].copy_from_slice(&music_crc.to_be_bytes());
    final_header[k + 154..k + 156].fill(0);
    let tag_crc = crc(&final_header[..190], 0, &table);
    final_header[k + 154..k + 156].copy_from_slice(&tag_crc.to_be_bytes());
    out.seek(SeekFrom::Start(0)).await?;
    out.write_all(&final_header).await?;
    out.sync_all().await?;
    Ok((path, u64::from(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires PAWKET_TEST_FFMPEG and at least two available cores"]
    async fn preserves_sample_count_and_chunk_boundaries() {
        let executable = PathBuf::from(std::env::var_os("PAWKET_TEST_FFMPEG").unwrap());
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("mono.wav");
        let samples = (BLOCK + OVERLAP) * SAMPLES + 97;
        let status = Command::new(&executable)
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=811:sample_rate=44100",
                "-af",
            ])
            .arg(format!("atrim=end_sample={samples}"))
            .arg(&src)
            .status()
            .await
            .unwrap();
        assert!(status.success());
        let (out, size) = run(&executable, &src, dir.path()).await.unwrap();
        assert_eq!(size, std::fs::metadata(&out).unwrap().len());
        let serial = dir.path().join("serial.mp3");
        assert!(Command::new(&executable)
            .args(["-v", "error", "-i"])
            .arg(&src)
            .args([
                "-c:a",
                "libmp3lame",
                "-reservoir",
                "0",
                "-compression_level",
                "7",
                "-b:a",
                "128k"
            ])
            .arg(&serial)
            .status()
            .await
            .unwrap()
            .success());
        let mut decoded = Vec::new();
        for path in [out.as_ref(), serial.as_path()] {
            let audio = Command::new(&executable)
                .args(["-v", "error", "-xerror", "-i"])
                .arg(path)
                .args(["-f", "s16le", "-"])
                .output()
                .await
                .unwrap();
            assert!(audio.status.success());
            assert_eq!(audio.stdout.len(), samples * 2);
            decoded.push(audio.stdout);
        }
        let center = BLOCK * SAMPLES * 2;
        let join = center - 4096..center + 4096;
        assert_eq!(decoded[0][join.clone()], decoded[1][join]);
    }
}
