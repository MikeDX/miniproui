//! Dump checks and MAME identification.
//!
//! `analyze` and `compare` cover the single-file and two-file checks from
//! MAME's `romcmp`. `mame_romident` shells out to `mame -romident`.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use sha1::{Digest, Sha1};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DumpReport {
    pub crc32: String,
    pub sha1: String,
    pub length: u64,
    pub lines: Vec<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RomIdentReport {
    pub lines: Vec<String>,
}

pub fn analyze(data: &[u8]) -> DumpReport {
    let mut lines = Vec::new();
    if data.is_empty() {
        lines.push("The buffer is empty.".into());
    } else if let Some(fill) = uniform_fill(data) {
        lines.push(format!(
            "Every byte is {fill:02X}. An all-FF read is a blank EPROM or data pins that are not connected. A blank 2764 (8192 x FF) is CRC32 b4293435, and mame -romident will match that image to blank graphics ROMs."
        ));
    } else {
        let (stuck_high, stuck_low) = stuck_bits(data);
        if stuck_high != 0 || stuck_low != 0 {
            lines.push(format!(
                "Stuck bits:{}{}.",
                bit_list(" always 1", stuck_high),
                bit_list(" always 0", stuck_low)
            ));
        }
        if data.len() % 2 == 0 && data[..data.len() / 2] == data[data.len() / 2..] {
            lines.push("The two halves of the dump are identical.".into());
        }
        if lines.is_empty() {
            lines.push("No blank fill, stuck bits, or repeated halves.".into());
        }
    }
    DumpReport {
        crc32: format!("{:08x}", crc32(data)),
        sha1: sha1_hex(data),
        length: data.len() as u64,
        lines,
    }
}

pub fn compare(dump: &[u8], other: &[u8], other_name: &str) -> Vec<String> {
    if dump.is_empty() {
        return vec!["The buffer is empty.".into()];
    }
    if other.is_empty() {
        return vec![format!("{other_name} is empty.")];
    }
    let mut lines = Vec::new();
    if dump.len() != other.len() {
        lines.push(format!(
            "Buffer is {} bytes; {other_name} is {} bytes.",
            dump.len(),
            other.len()
        ));
    }
    let n = dump.len().min(other.len());
    let same = dump[..n]
        .iter()
        .zip(other[..n].iter())
        .filter(|(left, right)| left == right)
        .count();
    if dump.len() == other.len() && same == n {
        lines.push(format!("Identical to {other_name}."));
        return lines;
    }
    let pct = if n == 0 {
        0.0
    } else {
        (same as f64) * 100.0 / (n as f64)
    };
    lines.push(format!(
        "{same} of {n} bytes match {other_name} ({pct:.2}%)."
    ));
    let mut diff_bits = 0u8;
    for (left, right) in dump[..n].iter().zip(other[..n].iter()) {
        diff_bits |= left ^ right;
    }
    if diff_bits != 0 {
        lines.push(format!("Differing bits:{}.", bit_list("", diff_bits)));
    }
    lines
}

pub fn mame_romident(path: &Path) -> Result<RomIdentReport, String> {
    let mame = find_mame()?;
    let output = Command::new(&mame)
        .arg("-romident")
        .arg(path)
        .output()
        .map_err(|err| format!("cannot run {}: {err}", mame.display()))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut lines: Vec<String> = stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("Identifying "))
        .map(collapse_ws)
        .take(40)
        .collect();
    let total = stdout
        .lines()
        .filter(|line| line.contains('=') || line.contains("NO MATCH"))
        .count();
    if total > lines.len() {
        lines.push(format!("{} more matches not shown.", total - lines.len()));
    }
    if lines.is_empty() {
        let err = String::from_utf8_lossy(&output.stderr);
        let err = err.trim();
        if err.is_empty() {
            lines.push("mame -romident returned no result.".into());
        } else {
            lines.push(err.to_string());
        }
    }
    Ok(RomIdentReport { lines })
}

fn find_mame() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("MAME_BIN") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
    }
    for candidate in ["/opt/homebrew/bin/mame", "/usr/local/bin/mame"] {
        let path = PathBuf::from(candidate);
        if path.is_file() {
            return Ok(path);
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join("mame");
            if candidate.is_file() {
                return Ok(candidate);
            }
            let candidate = dir.join("mame.exe");
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err("mame was not found. Install it with `brew install mame`, or set MAME_BIN.".into())
}

fn uniform_fill(data: &[u8]) -> Option<u8> {
    let first = *data.first()?;
    if data.iter().all(|byte| *byte == first) {
        Some(first)
    } else {
        None
    }
}

fn stuck_bits(data: &[u8]) -> (u8, u8) {
    let mut always_one = 0xffu8;
    let mut seen = 0u8;
    for byte in data {
        always_one &= byte;
        seen |= byte;
    }
    (always_one, !seen)
}

fn bit_list(label: &str, bits: u8) -> String {
    if bits == 0 {
        return String::new();
    }
    let list = (0..8)
        .filter(|bit| bits & (1 << bit) != 0)
        .map(|bit| bit.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    format!("{label} {list}")
}

fn crc32(data: &[u8]) -> u32 {
    crc::Crc::<u32>::new(&crc::CRC_32_ISO_HDLC).checksum(data)
}

fn sha1_hex(data: &[u8]) -> String {
    hex_bytes(&Sha1::digest(data))
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

fn collapse_ws(line: &str) -> String {
    line.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_2764_is_the_known_crc() {
        let data = vec![0xffu8; 8192];
        let report = analyze(&data);
        assert_eq!(report.crc32, "b4293435");
        assert!(report.lines[0].contains("Every byte is FF"));
    }

    #[test]
    fn stuck_bit_and_compare() {
        let dump: Vec<u8> = (0..16)
            .map(|index| if index % 2 == 0 { 0x01 } else { 0x03 })
            .collect();
        let report = analyze(&dump);
        assert!(report.lines.iter().any(|line| line.contains("always 1")));
        let other = vec![0x00u8; 16];
        let lines = compare(&dump, &other, "ref.bin");
        assert!(lines.iter().any(|line| line.contains("0 of 16")));
        assert!(lines.iter().any(|line| line.contains("Differing bits")));
        let same = compare(&dump, &dump, "ref.bin");
        assert!(same[0].contains("Identical"));
    }
}
