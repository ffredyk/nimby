//! Shared mod store: one copy of every downloaded file, hardlinked into any
//! workspace that needs it. Behaves like the Workshop cache but is fillable by
//! Nimby itself.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::Result;
use crate::formats::ModFile;
use crate::util::join_rel;
use crate::verify::{check_file, FileStatus};

#[derive(Clone, Debug)]
pub struct SharedStore {
    root: PathBuf,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct StoreStats {
    pub path: String,
    pub mods: usize,
    pub files: usize,
    pub bytes: u64,
}

impl SharedStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Folder for one mod inside the store, for example `<root>/@ace`.
    pub fn mod_dir(&self, mod_name: &str) -> PathBuf {
        self.root.join(sanitize_mod_name(mod_name))
    }

    pub fn file_path(&self, mod_name: &str, rel: &str) -> Result<PathBuf> {
        join_rel(&self.mod_dir(mod_name), rel)
    }

    /// Path to an existing, verified copy of `file`, if the store has one.
    pub fn find(&self, mod_name: &str, file: &ModFile) -> Option<PathBuf> {
        let path = self.file_path(mod_name, &file.path).ok()?;
        if check_file(file, &path) == FileStatus::Ok {
            Some(path)
        } else {
            None
        }
    }

    /// Directory where a mod's missing files should be written.
    pub fn prepare_mod_dir(&self, mod_name: &str) -> Result<PathBuf> {
        let dir = self.mod_dir(mod_name);
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    pub fn stats(&self) -> Result<StoreStats> {
        let mut stats = StoreStats {
            path: self.root.display().to_string(),
            ..Default::default()
        };
        if !self.root.is_dir() {
            return Ok(stats);
        }
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let mut mod_files = 0usize;
            for file in walkdir::WalkDir::new(entry.path()).follow_links(false) {
                let Ok(file) = file else { continue };
                if !file.file_type().is_file() {
                    continue;
                }
                mod_files += 1;
                stats.bytes += file.metadata().map(|m| m.len()).unwrap_or(0);
            }
            if mod_files > 0 {
                stats.mods += 1;
                stats.files += mod_files;
            }
        }
        Ok(stats)
    }

    /// Delete files that no longer belong to any mod folder on disk.
    pub fn prune_empty(&self) -> Result<usize> {
        let mut removed = 0usize;
        if !self.root.is_dir() {
            return Ok(removed);
        }
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let mut empty = true;
            for file in walkdir::WalkDir::new(entry.path()).follow_links(false) {
                let Ok(file) = file else { continue };
                if file.file_type().is_file() {
                    empty = false;
                    break;
                }
            }
            if empty {
                fs::remove_dir_all(entry.path())?;
                removed += 1;
            }
        }
        Ok(removed)
    }
}

/// Store folder names must stay predictable and safe.
pub fn sanitize_mod_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '@' | '-' | '_' | '.') {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() || out == "." || out == ".." {
        "_".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::{generate_from_dir, FileKind};

    fn source_mod() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let mod_dir = dir.path().join("@demo");
        fs::create_dir_all(mod_dir.join("addons")).unwrap();
        fs::write(mod_dir.join("mod.cpp"), b"name=\"demo\";\n").unwrap();
        fs::write(mod_dir.join("addons/a.pbo"), b"payload").unwrap();
        (dir, mod_dir)
    }

    #[test]
    fn finds_only_verified_files() {
        let (_guard, mod_dir) = source_mod();
        let manifest = generate_from_dir("@demo", &mod_dir).unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = SharedStore::new(store_dir.path());

        let file = &manifest.files[0];
        assert!(store.find("@demo", file).is_none());

        let dest = store.file_path("@demo", &file.path).unwrap();
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        fs::copy(mod_dir.join(&file.path), &dest).unwrap();
        assert!(store.find("@demo", file).is_some());

        // A file with the wrong content must not be reused.
        fs::write(&dest, b"tampered").unwrap();
        assert!(store.find("@demo", file).is_none());
    }

    #[test]
    fn stats_count_mods_files_and_bytes() {
        let (_guard, mod_dir) = source_mod();
        let manifest = generate_from_dir("@demo", &mod_dir).unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = SharedStore::new(store_dir.path());

        for file in &manifest.files {
            let dest = store.file_path("@demo", &file.path).unwrap();
            fs::create_dir_all(dest.parent().unwrap()).unwrap();
            let src = mod_dir.join(&file.path);
            fs::copy(&src, &dest).unwrap();
        }

        let stats = store.stats().unwrap();
        assert_eq!(stats.mods, 1);
        assert_eq!(stats.files, manifest.files.len());
        assert_eq!(stats.bytes, manifest.total_size());
        assert_eq!(manifest.files[0].kind, FileKind::File);
    }

    #[test]
    fn sanitizes_folder_names() {
        assert_eq!(sanitize_mod_name("@cba_ws"), "@cba_ws");
        assert_eq!(sanitize_mod_name("@my mod"), "@my_mod");
        assert_eq!(sanitize_mod_name(".."), "_");
        assert_eq!(sanitize_mod_name(""), "_");
    }

    #[test]
    fn refuses_path_traversal() {
        let store = SharedStore::new(std::env::temp_dir());
        assert!(store.file_path("@demo", "../escape").is_err());
    }
}
