//! Repository graphics (banner and icon), cached on disk so the UI can show
//! each repo's own artwork.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::checksum::sha1_hex;
use crate::config::normalize_url;
use crate::error::Result;
use crate::http::HttpSource;

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoImages {
    /// Absolute path of the cached banner, if the repo ships one.
    pub repo_image: Option<String>,
    /// Absolute path of the cached icon, if the repo ships one.
    pub icon_image: Option<String>,
    /// SHA1 of the banner, matching repo.json `imageChecksum`.
    pub repo_sha1: Option<String>,
    pub icon_sha1: Option<String>,
    pub cache_dir: String,
}

pub fn cache_dir_for(repo_url: &str, base: &Path) -> PathBuf {
    let key = sha1_hex(normalize_url(repo_url).as_bytes());
    base.join("images").join(&key[..16])
}

fn looks_like_image(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x89, b'P', b'N', b'G'])
        || bytes.starts_with(&[0xFF, 0xD8, 0xFF])
        || bytes.starts_with(b"GIF8")
        || (bytes.len() > 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP")
        || bytes.starts_with(b"BM")
}

/// Download (or reuse) `repo.png` / `icon.png` for a repo.
///
/// Missing images are not an error: Swifty repos treat them as optional.
pub fn fetch_images(http: &HttpSource, base: &Path) -> Result<RepoImages> {
    let cache = cache_dir_for(http.base(), base);
    fs::create_dir_all(&cache)?;

    let mut images = RepoImages {
        cache_dir: cache.display().to_string(),
        ..Default::default()
    };

    for (names, slot, sha_slot) in [
        (["repo.png", "repo.jpg"], 0usize, 0usize),
        (["icon.png", "icon.jpg"], 1usize, 1usize),
    ] {
        for name in names {
            let Ok(bytes) = http.get(name) else { continue };
            if !looks_like_image(&bytes) {
                continue;
            }
            let target = cache.join(name);
            fs::write(&target, &bytes)?;
            let sha1 = sha1_hex(&bytes);
            match slot {
                0 => {
                    images.repo_image = Some(target.display().to_string());
                    images.repo_sha1 = Some(sha1);
                }
                _ => {
                    images.icon_image = Some(target.display().to_string());
                    images.icon_sha1 = Some(sha1);
                }
            }
            let _ = sha_slot;
            break;
        }
    }

    Ok(images)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_dir_is_stable_and_url_normalized() {
        let base = Path::new("C:/cache");
        let a = cache_dir_for("https://example.com/repo", base);
        let b = cache_dir_for("https://example.com/repo/", base);
        assert_eq!(a, b);
    }

    #[test]
    fn detects_image_magic() {
        assert!(looks_like_image(&[0x89, b'P', b'N', b'G', 0, 0]));
        assert!(looks_like_image(&[0xFF, 0xD8, 0xFF, 0xE0]));
        assert!(!looks_like_image(b"<html>"));
        assert!(!looks_like_image(b""));
    }
}
