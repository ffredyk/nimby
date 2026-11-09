use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek};
use std::path::Path;

use crate::checksum::{md5_range, PBO_TRAILER_LEN};
use crate::error::{Error, Result};
use crate::formats::FilePart;

pub const PBO_MAGIC: [u8; 5] = [0, b's', b'r', b'e', b'V'];
/// Header block between magic and property list (version + reserved dwords).
const HEADER_DWORDS: u64 = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PboEntry {
    pub name: String,
    /// 0 = stored, 0x43707273 = CWS compressed.
    pub method: u32,
    pub original_size: u32,
    pub offset: u32,
    pub timestamp: u32,
    pub data_size: u32,
}

impl PboEntry {
    pub fn is_compressed(&self) -> bool {
        self.method != 0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PboInfo {
    pub data_start: u64,
    pub entries: Vec<PboEntry>,
    pub trailer: u64,
}

fn read_cstring<R: BufRead>(reader: &mut R) -> Result<String> {
    let mut buf = Vec::new();
    let read = reader.read_until(0, &mut buf)?;
    if read == 0 {
        return Err(Error::Manifest("unexpected end of PBO header".into()));
    }
    if buf.last() == Some(&0) {
        buf.pop();
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

fn read_u32<R: BufRead>(reader: &mut R) -> Result<u32> {
    let mut buf = [0u8; 4];
    reader.read_exact(&mut buf)?;
    Ok(u32::from_le_bytes(buf))
}

/// Parse the PBO header. Works for stored and CWS compressed PBOs; the data
/// section is treated as raw bytes, which is what Swifty hashes.
pub fn read_info(path: &Path) -> Result<PboInfo> {
    let file = File::open(path)?;
    let file_len = file.metadata()?.len();
    let mut reader = BufReader::with_capacity(64 * 1024, file);

    let mut magic = [0u8; 5];
    reader.read_exact(&mut magic)?;
    if magic != PBO_MAGIC {
        return Err(Error::Manifest(format!(
            "{} is not a PBO (bad magic)",
            path.display()
        )));
    }
    for _ in 0..HEADER_DWORDS {
        read_u32(&mut reader)?;
    }

    // Property list: name\0 value\0 ... terminated by an empty name.
    let mut property_guard = 0usize;
    loop {
        let name = read_cstring(&mut reader)?;
        if name.is_empty() {
            break;
        }
        let _value = read_cstring(&mut reader)?;
        property_guard += 1;
        if property_guard > 4096 {
            return Err(Error::Manifest(
                "PBO property list does not terminate".into(),
            ));
        }
    }

    // File entries: name\0 + 5 dwords, terminated by an empty name + zero dwords.
    let mut entries = Vec::new();
    loop {
        let name = read_cstring(&mut reader)?;
        let method = read_u32(&mut reader)?;
        let original_size = read_u32(&mut reader)?;
        let offset = read_u32(&mut reader)?;
        let timestamp = read_u32(&mut reader)?;
        let data_size = read_u32(&mut reader)?;
        if name.is_empty()
            && method == 0
            && original_size == 0
            && offset == 0
            && timestamp == 0
            && data_size == 0
        {
            break;
        }
        if entries.len() > 1_000_000 {
            return Err(Error::Manifest("PBO entry list does not terminate".into()));
        }
        entries.push(PboEntry {
            name,
            method,
            original_size,
            offset,
            timestamp,
            data_size,
        });
    }

    let data_start = reader.stream_position()?;
    let data_len: u64 = entries.iter().map(|e| e.data_size as u64).sum();
    let trailer = file_len
        .checked_sub(data_start)
        .and_then(|v| v.checked_sub(data_len))
        .ok_or_else(|| Error::Manifest("PBO data section overflows file size".into()))?;

    Ok(PboInfo {
        data_start,
        entries,
        trailer,
    })
}

/// Build Swifty parts for a PBO: `$$HEADER$$`, one part per entry, `$$END$$`.
pub fn parts(path: &Path) -> Result<Vec<FilePart>> {
    let info = read_info(path)?;
    let mut parts = Vec::with_capacity(info.entries.len() + 2);

    parts.push(FilePart {
        path: "$$HEADER$$".into(),
        start: 0,
        length: info.data_start,
        checksum: md5_range(path, 0, info.data_start)?,
    });

    let mut running = info.data_start;
    for entry in &info.entries {
        let start = if entry.offset != 0 {
            info.data_start + entry.offset as u64
        } else {
            running
        };
        let length = entry.data_size as u64;
        parts.push(FilePart {
            path: entry.name.clone(),
            start,
            length,
            checksum: md5_range(path, start, length)?,
        });
        running = start + length;
    }

    if info.trailer == PBO_TRAILER_LEN {
        let start = info.data_start + info.entries.iter().map(|e| e.data_size as u64).sum::<u64>();
        parts.push(FilePart {
            path: "$$END$$".into(),
            start,
            length: PBO_TRAILER_LEN,
            checksum: md5_range(path, start, PBO_TRAILER_LEN)?,
        });
    }

    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_pbo(entries: &[(&str, &[u8])], trailer: bool) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&PBO_MAGIC);
        out.extend_from_slice(&[0u8; 16]);
        out.extend_from_slice(b"prefix\x00x\\test\\addons\\x\x00");
        out.push(0);
        for (name, data) in entries {
            out.extend_from_slice(name.as_bytes());
            out.push(0);
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        }
        out.push(0);
        out.extend_from_slice(&[0u8; 20]);
        for (_, data) in entries {
            out.extend_from_slice(data);
        }
        if trailer {
            out.push(0);
            out.extend_from_slice(&[0xABu8; 20]);
        }
        out
    }

    #[test]
    fn reads_entries_and_trailer() {
        let pbo = build_pbo(&[("a.txt", b"hello"), ("sub/b.bin", b"world!!")], true);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.pbo");
        std::fs::write(&path, &pbo).unwrap();

        let info = read_info(&path).unwrap();
        assert_eq!(info.entries.len(), 2);
        assert_eq!(info.entries[0].name, "a.txt");
        assert_eq!(info.entries[1].name, "sub/b.bin");
        assert_eq!(info.trailer, PBO_TRAILER_LEN);
        assert_eq!(info.data_start + 5 + 7 + PBO_TRAILER_LEN, pbo.len() as u64);
    }

    #[test]
    fn generates_parts_matching_ranges() {
        let pbo = build_pbo(&[("a.txt", b"hello")], false);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.pbo");
        std::fs::write(&path, &pbo).unwrap();

        let parts = parts(&path).unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].path, "$$HEADER$$");
        assert_eq!(parts[0].start, 0);
        assert_eq!(parts[1].path, "a.txt");
        assert_eq!(parts[1].length, 5);
        assert_eq!(parts[1].checksum, crate::checksum::md5_hex(b"hello"));
        // contiguous
        assert_eq!(parts[0].length, parts[1].start);
    }
}
