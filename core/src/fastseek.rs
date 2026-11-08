//! Bounded random-access frame locator for long native RF FLAC captures.
//!
//! The MISRC recorder keeps the actual sample count in Vorbis RF metadata, but
//! stores a /1000 total in STREAMINFO. libFLAC's normal seek uses that incorrect
//! header. Instead, binary-search the *compressed* file using CRC-checked FLAC
//! frame headers and their absolute frame/sample numbers. No full-file scan,
//! persistent index, temporary PCM file, or source mutation is required.
//!
//! The returned header can be chained with a file handle positioned at
//! frame_offset and passed to claxon for sequential decode of only the cut
//! window. The usual SoX trim gets a frame-relative sample offset.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::probe::{self, MAX_HEADER_LEN};

const SCAN_BYTES: usize = 512 * 1024;
const HEADER_LIMIT: u64 = 64 * 1024 * 1024;
const FINAL_RANGE: u64 = 2 * SCAN_BYTES as u64;

#[derive(Debug, Clone, Copy)]
struct FramePosition {
    byte_offset: u64,
    sample_offset: u64,
    number: u64,
    block_samples: u64,
    variable: bool,
}

/// The prefix is the original FLAC metadata chain. It has no audio samples;
/// the decoder starts with the complete FLAC frame at `frame_offset`.
pub(crate) struct SeekWindow {
    pub(crate) header: Vec<u8>,
    pub(crate) frame_offset: u64,
    pub(crate) first_sample: u64,
}

fn frame_at(bytes: &[u8], at: usize, base: u64, block_size: u64) -> Option<FramePosition> {
    let (count, _, number, variable) = probe::parse_frame_header(bytes.get(at..)?)?;
    if u64::from(count) > block_size || count == 0 {
        return None;
    }
    let sample_offset = if variable {
        number
    } else {
        number.checked_mul(block_size)?
    };
    Some(FramePosition {
        byte_offset: base.checked_add(at as u64)?,
        sample_offset,
        number,
        block_samples: u64::from(count),
        variable,
    })
}

fn consecutive(a: &FramePosition, b: &FramePosition) -> bool {
    a.variable == b.variable
        && a.byte_offset < b.byte_offset
        && if a.variable {
            a.sample_offset.checked_add(a.block_samples) == Some(b.sample_offset)
        } else {
            a.number.checked_add(1) == Some(b.number)
        }
}

/// Read at most 512 KiB in a single seek. Require two successive, CRC-valid
/// frame headers so a matching byte pattern inside compressed data cannot
/// accidentally become an accepted entry point.
fn first_frame_after(
    file: &mut File,
    at: u64,
    file_size: u64,
    block_size: u64,
) -> Result<Option<FramePosition>, String> {
    if at >= file_size {
        return Ok(None);
    }
    let read_len = (file_size - at).min(SCAN_BYTES as u64) as usize;
    if read_len < MAX_HEADER_LEN {
        return Ok(None);
    }
    let mut buf = vec![0u8; read_len];
    file.seek(SeekFrom::Start(at)).map_err(|e| format!("FLAC seek failed: {e}"))?;
    file.read_exact(&mut buf)
        .map_err(|e| format!("FLAC seek window read failed: {e}"))?;

    let mut positions: Vec<FramePosition> = Vec::new();
    for i in 0..=read_len - MAX_HEADER_LEN {
        if buf[i] != 0xff || buf[i + 1] != 0xf8 && buf[i + 1] != 0xf9 {
            continue;
        }
        if let Some(p) = frame_at(&buf, i, at, block_size) {
            for prev in positions.iter().rev().take(8) {
                if consecutive(prev, &p) {
                    return Ok(Some(*prev));
                }
            }
            positions.push(p);
        }
    }
    Ok(None)
}

/// Locate a FLAC frame no later than `target_sample` with O(log(file_size))
/// small, random reads. False frame syncs are rejected by CRC-8 and successive
/// frame/sample numbering. We always prefer an earlier verified frame to
/// guessing: the SoX trim discards any remaining samples exactly.
pub(crate) fn locate(path: &Path, target_sample: u64) -> Result<SeekWindow, String> {
    let info = probe::probe(path);
    if !info.ok || info.audio_offset == 0 {
        return Err(format!("RF FLAC seek probe failed: {}", info.error));
    }
    if !info.is_rf || info.format != probe::InputFormat::Flac {
        return Err("fast RF seek requires native RF FLAC".into());
    }
    if info.total_samples_known && target_sample >= info.total_samples {
        return Err(format!(
            "cut start {target_sample} is after the known end ({} samples)",
            info.total_samples
        ));
    }
    let mut file = File::open(path).map_err(|e| format!("open RF FLAC failed: {e}"))?;
    let mut first_block = [0u8; 12];
    file.read_exact(&mut first_block).map_err(|e| e.to_string())?;
    if &first_block[..4] != b"fLaC" || first_block[4] & 0x7f != 0 {
        return Err("not a FLAC STREAMINFO header".into());
    }
    let fixed_block_size = u64::from(u16::from_be_bytes([first_block[10], first_block[11]]));
    if fixed_block_size == 0 {
        return Err("RF FLAC STREAMINFO has no maximum block size".into());
    }

    let mut best = first_frame_after(
        &mut file, info.audio_offset, info.file_size, fixed_block_size,
    )?.ok_or("no consecutive FLAC frame headers at the start of the stream")?;
    if best.sample_offset != 0 {
        return Err(format!(
            "first RF FLAC frame starts at sample {}; cannot establish absolute offsets",
            best.sample_offset
        ));
    }

    let mut lower = best.byte_offset;
    let mut upper = info.file_size;
    // Limit iterations as a guard against malformed or non-monotonic frame
    // numbering. A 96 GB capture converges in fewer than 20 probes.
    for _ in 0..48 {
        if upper <= lower.saturating_add(FINAL_RANGE) {
            break;
        }
        let middle = lower + (upper - lower) / 2;
        let found = first_frame_after(
            &mut file, middle, info.file_size, fixed_block_size,
        )?.ok_or_else(|| format!(
            "cannot find consecutive FLAC frame headers within 512 KiB of byte {middle}"
        ))?;
        if found.sample_offset <= target_sample {
            if found.sample_offset > best.sample_offset {
                best = found;
            }
            if found.byte_offset <= lower {
                break;
            }
            lower = found.byte_offset;
        } else {
            if found.byte_offset >= upper {
                break;
            }
            upper = found.byte_offset;
        }
    }

    // Narrow the remaining 1 MiB range. If the final scan fails (e.g. the
    // last frame is shorter), the already verified earlier frame is safe.
    let scan_start = lower;
    if let Some(p) = first_frame_after(
        &mut file, scan_start, info.file_size, fixed_block_size,
    )? {
        if p.sample_offset <= target_sample && p.sample_offset > best.sample_offset {
            best = p;
        }
    }

    if info.audio_offset > HEADER_LIMIT {
        return Err(format!(
            "FLAC metadata chain is unexpectedly large ({} bytes)",
            info.audio_offset
        ));
    }
    let mut header = vec![0u8; info.audio_offset as usize];
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    file.read_exact(&mut header)
        .map_err(|e| format!("FLAC metadata read failed: {e}"))?;

    Ok(SeekWindow {
        header,
        frame_offset: best.byte_offset,
        first_sample: best.sample_offset,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adjacent_fixed_and_variable_headers_are_checked() {
        let a = FramePosition {
            byte_offset: 300, sample_offset: 8192, number: 2,
            block_samples: 4096, variable: false,
        };
        let mut b = FramePosition { byte_offset: 600, sample_offset: 12288,
            number: 3, block_samples: 4096, variable: false };
        assert!(consecutive(&a, &b));
        b.number = 5;
        assert!(!consecutive(&a, &b));
        let variable_a = FramePosition { variable: true, number: 8192, ..a };
        let variable_b = FramePosition { variable: true, number: 12288, ..b };
        assert!(consecutive(&variable_a, &variable_b));
    }
}
