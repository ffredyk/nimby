use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, Result};
use crate::formats::{ModManifest, SrfFormat};
use crate::util::strip_bom;

/// Byte accounting for one installed mod, used to report disk savings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LockStats {
    pub logical_bytes: u64,
    pub workshop_bytes: u64,
    pub store_bytes: u64,
    pub downloaded_bytes: u64,
    pub copied_bytes: u64,
}

/// Provenance recorded next to every installed mod.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Lock {
    pub mod_name: String,
    pub checksum: String,
    /// Repository this mod came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// `dir-link`, `overlay`, `copy` or `passthrough`.
    pub mode: String,
    /// Full `mod.srf` (v3 JSON) so verify/repair work offline, and so
    /// directory-linked mods still have a manifest even though their folder
    /// belongs to Steam.
    pub manifest: Value,
    /// Per-file origin: `workshop:<path>`, `store:<path>`, `download:<url>`, ...
    #[serde(default)]
    pub files: BTreeMap<String, String>,
    #[serde(default)]
    pub stats: LockStats,
}

pub fn lock_path(target: &Path) -> Option<PathBuf> {
    let parent = target.parent()?;
    let name = target.file_name()?.to_string_lossy().to_string();
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    Some(parent.join(".nimby").join(format!("{safe}.json")))
}

pub fn is_managed(target: &Path) -> bool {
    lock_path(target).map(|p| p.is_file()).unwrap_or(false)
}

pub fn write_lock_full(
    target: &Path,
    manifest: &ModManifest,
    repo: Option<&str>,
    mode: &str,
    files: BTreeMap<String, String>,
    stats: LockStats,
) -> Result<()> {
    let Some(path) = lock_path(target) else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let srf = manifest.to_v3_json();
    let value: Value = serde_json::from_str(strip_bom(&srf))?;
    let lock = Lock {
        mod_name: manifest.name.clone(),
        checksum: manifest.checksum.clone(),
        repo: repo.map(|r| r.to_string()),
        mode: mode.to_string(),
        manifest: value,
        files,
        stats,
    };
    fs::write(path, serde_json::to_string_pretty(&lock)?)?;
    Ok(())
}

pub fn write_lock(
    target: &Path,
    manifest: &ModManifest,
    mode: &str,
    files: BTreeMap<String, String>,
) -> Result<()> {
    write_lock_full(target, manifest, None, mode, files, LockStats::default())
}

pub fn read_lock(target: &Path) -> Option<Lock> {
    let path = lock_path(target)?;
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Manifest of an installed mod: `mod.srf` inside the folder when present,
/// otherwise the copy stored in the lock file.
pub fn installed_manifest(target: &Path) -> Result<ModManifest> {
    let srf = target.join(crate::formats::SRF_FILE_NAME);
    if srf.is_file() {
        return ModManifest::load(&srf);
    }
    if let Some(lock) = read_lock(target) {
        let text = serde_json::to_string(&lock.manifest)?;
        return ModManifest::parse_v3(&text);
    }
    Err(Error::Manifest(format!(
        "no {} and no .nimby lock entry for {}",
        crate::formats::SRF_FILE_NAME,
        target.display()
    )))
}

/// Install mode recorded for a mod, if any.
pub fn installed_mode(target: &Path) -> Option<String> {
    read_lock(target).map(|l| l.mode)
}

/// Directories under `root` that look like Arma mods.
pub fn mod_dirs(root: &Path) -> Result<Vec<PathBuf>> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(root)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && p.file_name()
                    .map(|n| n.to_string_lossy().starts_with('@'))
                    .unwrap_or(false)
        })
        .collect();
    dirs.sort();
    Ok(dirs)
}

/// Write a manifest both as `mod.srf` and into the lock file.
pub fn stamp(target: &Path, manifest: &ModManifest, format: SrfFormat, mode: &str) -> Result<()> {
    manifest.write_to(target, format)?;
    write_lock(target, manifest, mode, BTreeMap::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> ModManifest {
        ModManifest::parse("\u{feff}{\"Name\":\"@x\",\"Checksum\":\"AB\",\"Files\":[]}".as_bytes())
            .unwrap()
    }

    #[test]
    fn lock_round_trip_restores_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("@x");
        fs::create_dir_all(&target).unwrap();
        write_lock(&target, &manifest(), "dir-link", BTreeMap::new()).unwrap();

        assert!(is_managed(&target));
        let loaded = installed_manifest(&target).unwrap();
        assert_eq!(loaded.name, "@x");
        assert_eq!(loaded.checksum, "AB");
        assert_eq!(installed_mode(&target).as_deref(), Some("dir-link"));
    }

    #[test]
    fn srf_wins_over_lock() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("@x");
        fs::create_dir_all(&target).unwrap();
        write_lock(&target, &manifest(), "overlay", BTreeMap::new()).unwrap();
        fs::write(
            target.join("mod.srf"),
            "\u{feff}{\"Name\":\"@x\",\"Checksum\":\"CD\",\"Files\":[]}",
        )
        .unwrap();
        assert_eq!(installed_manifest(&target).unwrap().checksum, "CD");
    }
}
