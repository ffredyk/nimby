//! Persistent cache of file hashes, keyed by path plus size and modification
//! time. Verifying a large Workshop item costs hundreds of megabytes of hashing
//! per install; with the cache an unchanged file is one `stat` plus a lookup.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::formats::ModFile;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Entry {
    size: u64,
    /// Modification time in nanoseconds since the unix epoch.
    modified: u128,
    /// File checksum (MD5 over the part hashes), as stored in `mod.srf`.
    checksum: String,
}

#[derive(Debug, Default)]
pub struct HashCache {
    path: PathBuf,
    entries: HashMap<String, Entry>,
    dirty: bool,
    hits: u64,
    misses: u64,
}

fn key_for(path: &Path) -> String {
    let text = path.to_string_lossy().to_string();
    if cfg!(windows) {
        text.to_ascii_lowercase()
    } else {
        text
    }
}

fn modified_nanos(path: &Path) -> Option<u128> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    modified
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_nanos())
}

impl HashCache {
    /// In-memory cache only (tests, or when persistence is not wanted).
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// Load a cache from disk, tolerating missing or corrupt files.
    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let entries = fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<HashMap<String, Entry>>(&text).ok())
            .unwrap_or_default();
        Self {
            path,
            entries,
            dirty: false,
            hits: 0,
            misses: 0,
        }
    }

    pub fn hits(&self) -> u64 {
        self.hits
    }

    pub fn misses(&self) -> u64 {
        self.misses
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Cached checksum for a file, if the size and mtime still match.
    pub fn lookup(&mut self, path: &Path, size: u64) -> Option<String> {
        let modified = modified_nanos(path)?;
        let key = key_for(path);
        let entry = self.entries.get(&key)?;
        if entry.size == size && entry.modified == modified {
            self.hits += 1;
            Some(entry.checksum.clone())
        } else {
            None
        }
    }

    pub fn store(&mut self, path: &Path, size: u64, checksum: &str) {
        let Some(modified) = modified_nanos(path) else {
            return;
        };
        let key = key_for(path);
        let entry = Entry {
            size,
            modified,
            checksum: checksum.to_string(),
        };
        if let Some(existing) = self.entries.get(&key) {
            if existing.size == size
                && existing.modified == modified
                && existing.checksum.eq_ignore_ascii_case(checksum)
            {
                return;
            }
        }
        self.entries.insert(key, entry);
        self.dirty = true;
    }

    /// Drop entries whose file is gone; keeps the cache from growing forever.
    pub fn prune(&mut self) -> usize {
        let before = self.entries.len();
        self.entries.retain(|key, entry| match fs::metadata(key) {
            Ok(meta) => {
                meta.len() == entry.size && modified_nanos(Path::new(key)) == Some(entry.modified)
            }
            Err(_) => false,
        });
        let removed = before - self.entries.len();
        if removed > 0 {
            self.dirty = true;
        }
        self.misses += 0;
        removed
    }

    pub fn save(&mut self) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string(&self.entries)?;
        let temp = self.path.with_extension("json.tmp");
        fs::write(&temp, text)?;
        fs::rename(&temp, &self.path)?;
        self.dirty = false;
        Ok(())
    }
}

/// True when the cached checksum proves the file matches the manifest entry.
pub fn cache_confirms(cache: &mut HashCache, file: &ModFile, path: &Path, size: u64) -> bool {
    match cache.lookup(path, size) {
        Some(checksum) => checksum.eq_ignore_ascii_case(&file.checksum),
        None => {
            cache.misses += 1;
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_file(checksum: &str) -> ModFile {
        ModFile {
            path: "x.bin".into(),
            length: 0,
            checksum: checksum.into(),
            kind: crate::formats::FileKind::File,
            parts: vec![],
        }
    }

    #[test]
    fn caches_by_size_and_mtime() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a.bin");
        fs::write(&target, b"abc").unwrap();

        let mut cache = HashCache::in_memory();
        assert!(!cache_confirms(
            &mut cache,
            &manifest_file("AA"),
            &target,
            3
        ));

        cache.store(&target, 3, "AA");
        assert!(cache_confirms(&mut cache, &manifest_file("AA"), &target, 3));
        // Wrong checksum in the manifest is not confirmed.
        assert!(!cache_confirms(
            &mut cache,
            &manifest_file("BB"),
            &target,
            3
        ));
        // A different size is not confirmed.
        assert!(!cache_confirms(
            &mut cache,
            &manifest_file("AA"),
            &target,
            99
        ));
        assert_eq!(cache.hits(), 2);
    }

    #[test]
    fn rewrite_invalidates() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a.bin");
        fs::write(&target, b"abc").unwrap();

        let mut cache = HashCache::in_memory();
        cache.store(&target, 3, "AA");
        // Same size, new content: mtime changes on every real write.
        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(&target, b"abd").unwrap();
        assert!(!cache_confirms(
            &mut cache,
            &manifest_file("AA"),
            &target,
            3
        ));
    }

    #[test]
    fn persists_and_prunes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let target = dir.path().join("a.bin");
        fs::write(&target, b"abc").unwrap();

        let mut cache = HashCache::load(&path);
        cache.store(&target, 3, "AA");
        cache.save().unwrap();

        let mut reloaded = HashCache::load(&path);
        assert!(cache_confirms(
            &mut reloaded,
            &manifest_file("AA"),
            &target,
            3
        ));

        fs::remove_file(&target).unwrap();
        assert_eq!(reloaded.prune(), 1);
        assert!(reloaded.is_empty());
    }
}
