use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use md5::{Digest, Md5};

use crate::error::Result;
use crate::formats::{FilePart, ModFile};

/// Size of the trailing PBO signature block (1 flag byte + 20 byte sha1).
pub const PBO_TRAILER_LEN: u64 = 21;

pub fn md5_hex(bytes: &[u8]) -> String {
    let mut hasher = Md5::new();
    hasher.update(bytes);
    hex::encode_upper(hasher.finalize())
}

pub fn sha1_hex(bytes: &[u8]) -> String {
    use sha1::Sha1;
    let mut hasher = Sha1::new();
    hasher.update(bytes);
    hex::encode_upper(hasher.finalize())
}

/// MD5 of a byte range of a file, streamed.
pub fn md5_range(path: &Path, start: u64, len: u64) -> Result<String> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(start))?;
    let mut hasher = Md5::new();
    let mut remaining = len;
    let mut buf = vec![0u8; 1 << 20];
    while remaining > 0 {
        let take = std::cmp::min(buf.len() as u64, remaining) as usize;
        file.read_exact(&mut buf[..take])?;
        hasher.update(&buf[..take]);
        remaining -= take as u64;
    }
    Ok(hex::encode_upper(hasher.finalize()))
}

/// MD5 over the concatenated uppercase hex of every part hash (Swifty file hash).
pub fn file_checksum(parts: &[FilePart]) -> String {
    let mut hasher = Md5::new();
    for part in parts {
        hasher.update(part.checksum.as_bytes());
    }
    hex::encode_upper(hasher.finalize())
}

/// MD5 over `fileChecksum + path` for every file, sorted by path (Swifty v3 mod hash).
pub fn mod_checksum(files: &[ModFile]) -> String {
    let mut sorted: Vec<&ModFile> = files.iter().collect();
    sorted.sort_by(|a, b| a.path.cmp(&b.path));
    let mut hasher = Md5::new();
    for file in sorted {
        hasher.update(file.checksum.as_bytes());
        hasher.update(file.path.as_bytes());
    }
    hex::encode_upper(hasher.finalize())
}

/// Legacy swifty-cli mod hash: first 16 hex chars of each file hash, sorted by name.
pub fn mod_checksum_legacy(files: &[ModFile]) -> String {
    let mut sorted: Vec<&ModFile> = files.iter().collect();
    sorted.sort_by(|a, b| a.path.cmp(&b.path));
    let mut hasher = Md5::new();
    for file in sorted {
        let head: String = file.checksum.chars().take(16).collect();
        hasher.update(head.as_bytes());
    }
    hex::encode_upper(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::FileKind;

    fn part(path: &str, start: u64, length: u64, checksum: &str) -> FilePart {
        FilePart {
            path: path.to_string(),
            start,
            length,
            checksum: checksum.to_string(),
        }
    }

    #[test]
    fn file_hash_is_md5_of_part_hashes() {
        // Golden: @cba_ws mod.cpp, live repo.
        let parts = vec![part(
            "mod.cpp_895",
            0,
            895,
            "32DCD6F6EB27CF18E4917106356ADCED",
        )];
        assert_eq!(file_checksum(&parts), "0A266973E65CDA6F39BC5C140AC937FC");
    }

    fn file(path: &str, checksum: &str) -> ModFile {
        ModFile {
            path: path.into(),
            length: 0,
            checksum: checksum.into(),
            kind: FileKind::File,
            parts: vec![],
        }
    }

    #[test]
    fn mod_hash_is_md5_of_hash_plus_path() {
        // Golden: @cba_ws mod.srf (v3) from cdn.synixe.contractors.
        let files = vec![
            file(
                "keys/pallas_cba_ws.bikey",
                "9EACC8526973572A079036CDAD7B7D4F",
            ),
            file(
                "addons/lxws_cba_compat.pbo.pallas_cba_ws.bisign",
                "95B6615D84C303221887EE361A6474E1",
            ),
            file(
                "addons/lxws_cba_compat.pbo",
                "5670E8F32A56599E68CAD0ECD83B2380",
            ),
            file("mod.cpp", "0A266973E65CDA6F39BC5C140AC937FC"),
            file("meta.cpp", "C1FED3CD0020CF2A5793CB13F846930E"),
        ];
        assert_eq!(mod_checksum(&files), "DD39F3C34318C8CB5950E0B785FBCB08");
    }

    #[test]
    fn mod_hash_matches_second_live_sample() {
        // Golden: @cba_rf mod.srf (v3) from cdn.synixe.contractors.
        let files = vec![
            file(
                "keys/pallas_cba_rf.bikey",
                "C04F36CB2262D2D20C0E72CE8E854598",
            ),
            file(
                "addons/lxrf_cba_compat.pbo",
                "3384B79152DDE3BE899AB294BF5ACCE7",
            ),
            file(
                "addons/lxrf_cba_compat.pbo.pallas_cba_rf.bisign",
                "58B0426EBF9B739768AA13D0600EE31B",
            ),
            file("mod.cpp", "C2F5843A27057443C79947AB8A8619B5"),
            file("meta.cpp", "16BE2C1FDFB2E9D1DD0C14E366F7E4CA"),
        ];
        assert_eq!(mod_checksum(&files), "5AF55C4096FF52B52CC3F7106A04ACA1");
    }
}
