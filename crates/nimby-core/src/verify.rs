use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::checksum::{file_checksum, md5_range};
use crate::formats::{ModFile, ModManifest};
use crate::util::{join_rel, rel_path};
use crate::Result;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileStatus {
    Ok,
    Missing,
    WrongLength { expected: u64, actual: u64 },
    PartMismatch { part: String },
    ChecksumMismatch { expected: String, actual: String },
}

#[derive(Clone, Debug)]
pub struct FileCheck {
    pub path: String,
    pub status: FileStatus,
}

#[derive(Clone, Debug)]
pub struct VerifyReport {
    pub expected_checksum: String,
    pub actual_checksum: Option<String>,
    pub files: Vec<FileCheck>,
    pub extra_files: Vec<String>,
}

impl VerifyReport {
    pub fn files_ok(&self) -> bool {
        self.files.iter().all(|f| f.status == FileStatus::Ok)
    }

    /// Every manifest file matches. Extra files on disk are allowed: a linked
    /// Workshop folder legitimately contains files the repo does not ship.
    pub fn is_ok(&self) -> bool {
        self.files_ok()
    }

    /// Every manifest file matches and nothing extra is present.
    pub fn is_clean(&self) -> bool {
        self.files_ok() && self.extra_files.is_empty()
    }

    pub fn mismatches(&self) -> impl Iterator<Item = &FileCheck> {
        self.files.iter().filter(|f| f.status != FileStatus::Ok)
    }
}

/// Verify a single manifest file on disk, hashing every part.
pub fn check_file(file: &ModFile, path: &Path) -> FileStatus {
    check_file_shared(file, path, None)
}

/// Like [`check_file`], but consults and fills a hash cache. A cache hit means
/// the size and modification time still match what was hashed before, so the
/// (possibly huge) file is not read again.
pub fn check_file_shared(
    file: &ModFile,
    path: &Path,
    cache: Option<&std::sync::Mutex<crate::hashcache::HashCache>>,
) -> FileStatus {
    let Ok(meta) = fs::metadata(path) else {
        return FileStatus::Missing;
    };
    if !meta.is_file() {
        return FileStatus::Missing;
    }
    let actual = meta.len();
    if actual != file.length {
        return FileStatus::WrongLength {
            expected: file.length,
            actual,
        };
    }

    if let Some(cache) = cache {
        if let Ok(mut cache) = cache.lock() {
            if crate::hashcache::cache_confirms(&mut cache, file, path, actual) {
                return FileStatus::Ok;
            }
        }
    }

    for part in &file.parts {
        match md5_range(path, part.start, part.length) {
            Ok(hash) if hash.eq_ignore_ascii_case(&part.checksum) => {}
            Ok(_) => {
                return FileStatus::PartMismatch {
                    part: part.path.clone(),
                }
            }
            Err(_) => return FileStatus::Missing,
        }
    }
    let computed = file_checksum(&file.parts);
    if !computed.eq_ignore_ascii_case(&file.checksum) {
        return FileStatus::ChecksumMismatch {
            expected: file.checksum.clone(),
            actual: computed,
        };
    }
    if let Some(cache) = cache {
        if let Ok(mut cache) = cache.lock() {
            cache.store(path, actual, &computed);
        }
    }
    FileStatus::Ok
}

/// Verify a mod directory against its manifest.
pub fn verify_dir(manifest: &ModManifest, dir: &Path) -> Result<VerifyReport> {
    verify_dir_cached(manifest, dir, None)
}

/// Like [`verify_dir`], with an optional persistent hash cache.
pub fn verify_dir_cached(
    manifest: &ModManifest,
    dir: &Path,
    cache: Option<&std::sync::Mutex<crate::hashcache::HashCache>>,
) -> Result<VerifyReport> {
    let mut checks = Vec::with_capacity(manifest.files.len());
    let mut all_ok = true;

    for file in &manifest.files {
        let path = match join_rel(dir, &file.path) {
            Ok(p) => p,
            Err(_) => {
                all_ok = false;
                checks.push(FileCheck {
                    path: file.path.clone(),
                    status: FileStatus::Missing,
                });
                continue;
            }
        };
        let status = check_file_shared(file, &path, cache);
        if status != FileStatus::Ok {
            all_ok = false;
        }
        checks.push(FileCheck {
            path: file.path.clone(),
            status,
        });
    }

    let actual_checksum = if all_ok && checks.len() == manifest.files.len() {
        Some(manifest.checksum_for(manifest.format))
    } else {
        None
    };

    let mut extras = Vec::new();
    let known: BTreeSet<&str> = manifest.files.iter().map(|f| f.path.as_str()).collect();
    for entry in walkdir::WalkDir::new(dir).follow_links(false) {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_file() {
            continue;
        }
        let Some(rel) = rel_path(dir, entry.path()) else {
            continue;
        };
        if rel == crate::formats::SRF_FILE_NAME || rel.starts_with(".nimby/") {
            continue;
        }
        if !known.contains(rel.as_str()) {
            extras.push(rel);
        }
    }
    extras.sort();

    Ok(VerifyReport {
        expected_checksum: manifest.checksum.clone(),
        actual_checksum,
        files: checks,
        extra_files: extras,
    })
}

/// Quick check used by the link planner: does this file match byte for byte?
pub fn file_matches(file: &ModFile, path: &Path) -> bool {
    check_file(file, path) == FileStatus::Ok
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::{generate_from_dir, FileKind};

    #[test]
    fn verifies_generated_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let mod_dir = dir.path().join("@t");
        fs::create_dir_all(mod_dir.join("addons")).unwrap();
        fs::write(mod_dir.join("mod.cpp"), b"name=\"t\";\n").unwrap();
        fs::write(mod_dir.join("addons").join("a.pbo"), b"not-a-real-pbo").unwrap();

        let manifest = generate_from_dir("@t", &mod_dir).unwrap();
        let report = verify_dir(&manifest, &mod_dir).unwrap();
        assert!(report.is_ok(), "{:?}", report);
        assert_eq!(
            report.actual_checksum.as_deref(),
            Some(manifest.checksum.as_str())
        );

        fs::write(mod_dir.join("mod.cpp"), b"name=\"changed\";\n").unwrap();
        let report = verify_dir(&manifest, &mod_dir).unwrap();
        assert!(!report.files_ok());
        assert!(report.mismatches().any(|f| f.path == "mod.cpp"));
    }

    #[test]
    fn detects_extra_files() {
        let dir = tempfile::tempdir().unwrap();
        let mod_dir = dir.path().join("@t");
        fs::create_dir_all(&mod_dir).unwrap();
        fs::write(mod_dir.join("mod.cpp"), b"x").unwrap();
        let manifest = generate_from_dir("@t", &mod_dir).unwrap();
        fs::write(mod_dir.join("extra.txt"), b"x").unwrap();
        let report = verify_dir(&manifest, &mod_dir).unwrap();
        assert_eq!(report.extra_files, vec!["extra.txt".to_string()]);
        // Content is still correct; only strict checks complain about extras.
        assert!(report.is_ok());
        assert!(!report.is_clean());
        assert_eq!(manifest.files[0].kind, FileKind::File);
    }
}
