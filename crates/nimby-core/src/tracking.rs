//! Per-repository tracking index.
//!
//! `<workspace>/.nimby/tracking.json` records, for every installed file, what it
//! is and where it came from:
//!
//! - `origin`: downloaded from the repository, copied, taken from the Steam
//!   Workshop cache, or hardlinked from the shared store;
//! - `location`: whether the bytes live in the workspace, the store, or Steam;
//! - `size`/`mtime`: a stat signature, so an update check can spot a changed
//!   file without reading a single byte;
//! - `checksum`: the manifest checksum, so a *deep* check can confirm content.
//!
//! The index is what makes the Stats page accurate (unique vs. shared vs.
//! Workshop bytes) and what makes `nimby update` cheap: with it, a repository
//! with thousands of files can be compared without hashing them.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::formats::ModManifest;
use crate::verify::FileStatus;

/// Where the bytes of an installed file came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileOrigin {
    /// Fetched from the repository (the only bytes that cost bandwidth).
    Download,
    /// Read from a local mirror or copied because no link was possible.
    Copy,
    /// Taken from the Steam Workshop cache.
    Workshop,
    /// Taken from the shared mod store.
    Store,
    /// Kept from a previous run because it already matched.
    Reuse,
}

impl FileOrigin {
    pub fn as_str(&self) -> &'static str {
        match self {
            FileOrigin::Download => "download",
            FileOrigin::Copy => "copy",
            FileOrigin::Workshop => "workshop",
            FileOrigin::Store => "store",
            FileOrigin::Reuse => "reuse",
        }
    }

    /// Parse the `kind:path` provenance string the lock files use.
    pub fn from_lock_entry(entry: &str) -> Option<FileOrigin> {
        let (kind, _) = entry.split_once(':')?;
        match kind {
            "download" => Some(FileOrigin::Download),
            "copy" => Some(FileOrigin::Copy),
            "workshop" => Some(FileOrigin::Workshop),
            "store" => Some(FileOrigin::Store),
            "reuse" => Some(FileOrigin::Reuse),
            _ => None,
        }
    }
}

/// Where the bytes physically live.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileLocation {
    /// The mod folder itself holds the bytes (a copy, or a download without a store).
    Workspace,
    /// The shared store holds them; the mod folder has a hardlink.
    Store,
    /// Steam holds them; the mod folder has a hardlink or a junction.
    Workshop,
}

impl FileLocation {
    pub fn as_str(&self) -> &'static str {
        match self {
            FileLocation::Workspace => "workspace",
            FileLocation::Store => "store",
            FileLocation::Workshop => "workshop",
        }
    }
}

/// One tracked file of one mod.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FileTrack {
    pub bytes: u64,
    pub checksum: String,
    pub origin: FileOrigin,
    pub location: FileLocation,
    /// How the file is materialized: `hardlink`, `symlink`, `junction`, `copy`.
    pub link: String,
    /// Workshop path, store path or repository-relative path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Size and modification time when it was installed: a cheap change signal.
    pub size: u64,
    pub mtime: i64,
}

impl Default for FileTrack {
    fn default() -> Self {
        Self {
            bytes: 0,
            checksum: String::new(),
            origin: FileOrigin::Download,
            location: FileLocation::Workspace,
            link: String::new(),
            source: None,
            size: 0,
            mtime: 0,
        }
    }
}

/// Everything tracked for one installed mod.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ModTrack {
    pub checksum: String,
    pub repo: String,
    pub mode: String,
    /// Unix seconds.
    pub installed_at: i64,
    pub files: BTreeMap<String, FileTrack>,
}

/// Byte counts derived from a set of tracked files.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackTotals {
    pub files: usize,
    pub logical_bytes: u64,
    /// Bytes on the Steam side (linked, never copied).
    pub workshop_bytes: u64,
    /// Bytes in the shared store (hardlinked into the workspace).
    pub shared_bytes: u64,
    /// Bytes the workspace folder itself holds.
    pub workspace_bytes: u64,
    /// Bytes fetched from the repository, wherever they ended up.
    pub downloaded_bytes: u64,
    /// Bytes copied without a link.
    pub copied_bytes: u64,
}

impl TrackTotals {
    pub fn add(&mut self, file: &FileTrack) {
        self.files += 1;
        self.logical_bytes += file.bytes;
        match file.location {
            FileLocation::Workshop => self.workshop_bytes += file.bytes,
            FileLocation::Store => self.shared_bytes += file.bytes,
            FileLocation::Workspace => self.workspace_bytes += file.bytes,
        }
        match file.origin {
            FileOrigin::Download => self.downloaded_bytes += file.bytes,
            FileOrigin::Copy => self.copied_bytes += file.bytes,
            _ => {}
        }
    }

    /// Bytes that already existed elsewhere and were not written again.
    pub fn saved_bytes(&self) -> u64 {
        self.workshop_bytes + self.shared_bytes
    }
}

/// What a mod needs before it is current again.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdateState {
    /// Nothing changed.
    UpToDate,
    /// The repository ships a different mod checksum.
    Outdated,
    /// Files changed on disk (size/mtime no longer match).
    Drifted,
    /// Files are missing from the mod folder.
    Missing,
    /// No tracking entry yet, for example after migrating from Swifty.
    Untracked,
}

impl UpdateState {
    pub fn as_str(&self) -> &'static str {
        match self {
            UpdateState::UpToDate => "up_to_date",
            UpdateState::Outdated => "outdated",
            UpdateState::Drifted => "drifted",
            UpdateState::Missing => "missing",
            UpdateState::Untracked => "untracked",
        }
    }

    pub fn needs_work(&self) -> bool {
        !matches!(self, UpdateState::UpToDate)
    }
}

/// Result of comparing a mod on disk with its manifest and tracking entry.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackCheck {
    pub mod_name: String,
    pub state: UpdateState,
    pub tracked_checksum: Option<String>,
    pub manifest_checksum: String,
    /// Files whose stat signature changed (or that are missing) when the check
    /// was shallow, or files that fail their hash when it was deep.
    pub files: Vec<String>,
    /// How the check was performed.
    pub deep: bool,
}

impl TrackCheck {
    pub fn needs_work(&self) -> bool {
        self.state.needs_work()
    }
}

/// The tracking index of one install folder.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RepoIndex {
    pub version: u32,
    /// Unix seconds of the last write.
    pub updated_at: i64,
    /// Repository URL this folder belongs to, for diagnostics.
    pub repo: String,
    pub mods: BTreeMap<String, ModTrack>,
}

impl Default for RepoIndex {
    fn default() -> Self {
        Self {
            version: 1,
            updated_at: 0,
            repo: String::new(),
            mods: BTreeMap::new(),
        }
    }
}

pub const TRACKING_FILE: &str = "tracking.json";

pub fn tracking_path(workspace: &Path) -> PathBuf {
    workspace.join(".nimby").join(TRACKING_FILE)
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn signature(path: &Path) -> (u64, i64) {
    let Ok(meta) = fs::metadata(path) else {
        return (0, 0);
    };
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    (meta.len(), mtime)
}

impl RepoIndex {
    /// Read the index of a folder, or an empty one when it does not exist.
    pub fn load(workspace: &Path) -> Self {
        let path = tracking_path(workspace);
        match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(crate::util::strip_bom(&text)).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn load_result(workspace: &Path) -> Result<Self> {
        let path = tracking_path(workspace);
        if !path.is_file() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(&path)?;
        Ok(serde_json::from_str(crate::util::strip_bom(&text))?)
    }

    pub fn save(&mut self, workspace: &Path) -> Result<()> {
        self.updated_at = now();
        let path = tracking_path(workspace);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    /// Record (or replace) one mod from an install report.
    pub fn record(
        &mut self,
        workspace: &Path,
        repo: Option<&str>,
        manifest: &ModManifest,
        mode: &str,
        files: &[crate::install::InstalledFile],
    ) {
        let mut track = ModTrack {
            checksum: manifest.checksum.clone(),
            repo: repo.unwrap_or_default().to_string(),
            mode: mode.to_string(),
            installed_at: now(),
            files: BTreeMap::new(),
        };
        for file in files {
            let target = crate::util::join_rel(&workspace.join(&manifest.name), &file.path);
            let (size, mtime) = match &target {
                Ok(path) => signature(path),
                Err(_) => (file.bytes, 0),
            };
            track.files.insert(
                file.path.clone(),
                FileTrack {
                    bytes: file.bytes,
                    checksum: file.checksum.clone(),
                    origin: file.origin,
                    location: file.location,
                    link: file.link.clone(),
                    source: file.source.clone(),
                    size,
                    mtime,
                },
            );
        }
        self.mods.insert(manifest.name.clone(), track);
    }

    /// Rebuild the entry of one installed mod from its lock file, without any
    /// network traffic or hashing. Used to backfill folders installed before
    /// tracking existed.
    pub fn rebuild_mod(
        &mut self,
        workspace: &Path,
        manifest: &ModManifest,
        lock: &crate::workspace::Lock,
        store_root: Option<&Path>,
    ) {
        use crate::store::SharedStore;
        let mod_dir = workspace.join(&manifest.name);
        let store = store_root.map(SharedStore::new);
        let mut track = ModTrack {
            checksum: manifest.checksum.clone(),
            repo: lock.repo.clone().unwrap_or_default(),
            mode: lock.mode.clone(),
            installed_at: now(),
            files: BTreeMap::new(),
        };
        for file in &manifest.files {
            let entry = lock.files.get(&file.path).cloned().unwrap_or_default();
            let origin = FileOrigin::from_lock_entry(&entry).unwrap_or(FileOrigin::Reuse);
            let source = entry.split_once(':').map(|(_, path)| path.to_string());
            let location = match origin {
                FileOrigin::Workshop => FileLocation::Workshop,
                FileOrigin::Store => FileLocation::Store,
                _ => {
                    // A download can live in the store; the store copy is the
                    // authority when it exists.
                    let in_store = store
                        .as_ref()
                        .and_then(|store| store.file_path(&manifest.name, &file.path).ok())
                        .map(|path| path.is_file())
                        .unwrap_or(false);
                    if in_store {
                        FileLocation::Store
                    } else {
                        FileLocation::Workspace
                    }
                }
            };
            let (size, mtime) = crate::util::join_rel(&mod_dir, &file.path)
                .map(|path| signature(&path))
                .unwrap_or((0, 0));
            track.files.insert(
                file.path.clone(),
                FileTrack {
                    bytes: file.length,
                    checksum: file.checksum.clone(),
                    origin,
                    location,
                    link: "unknown".to_string(),
                    source,
                    size,
                    mtime,
                },
            );
        }
        self.mods.insert(manifest.name.clone(), track);
    }

    pub fn forget(&mut self, mod_name: &str) -> bool {
        self.mods.remove(mod_name).is_some()
    }

    pub fn totals(&self, mod_name: &str) -> TrackTotals {
        let mut totals = TrackTotals::default();
        if let Some(track) = self.mods.get(mod_name) {
            for file in track.files.values() {
                totals.add(file);
            }
        }
        totals
    }

    pub fn all_totals(&self) -> TrackTotals {
        let mut totals = TrackTotals::default();
        for track in self.mods.values() {
            for file in track.files.values() {
                totals.add(file);
            }
        }
        totals
    }

    /// Compare one installed mod with the manifest the repository ships.
    ///
    /// Shallow (default) compares sizes and modification times, which is enough
    /// to detect a changed file; deep hashes everything.
    pub fn check(
        &self,
        target: &Path,
        manifest: &ModManifest,
        deep: bool,
        cache: Option<&std::sync::Mutex<crate::hashcache::HashCache>>,
    ) -> TrackCheck {
        let tracked = self.mods.get(&manifest.name);
        let mut files = Vec::new();
        let mut state = if tracked.is_none() {
            UpdateState::Untracked
        } else if tracked
            .map(|t| !t.checksum.eq_ignore_ascii_case(&manifest.checksum))
            .unwrap_or(false)
        {
            UpdateState::Outdated
        } else {
            UpdateState::UpToDate
        };

        for file in &manifest.files {
            let path = match crate::util::join_rel(target, &file.path) {
                Ok(path) => path,
                Err(_) => {
                    files.push(file.path.clone());
                    continue;
                }
            };
            match &tracked.and_then(|t| t.files.get(&file.path)) {
                None => {
                    if !path.is_file() {
                        files.push(file.path.clone());
                        if state == UpdateState::UpToDate {
                            state = UpdateState::Missing;
                        }
                    }
                }
                Some(entry) => {
                    if deep {
                        let ok =
                            crate::verify::check_file_shared(file, &path, cache) == FileStatus::Ok;
                        if !ok {
                            files.push(file.path.clone());
                            if state == UpdateState::UpToDate || state == UpdateState::Untracked {
                                state = UpdateState::Drifted;
                            }
                        }
                        continue;
                    }
                    let (size, mtime) = signature(&path);
                    if !path.is_file() {
                        files.push(file.path.clone());
                        if !state.needs_work() || state == UpdateState::Untracked {
                            state = UpdateState::Missing;
                        }
                    } else if size != entry.size || (entry.mtime != 0 && mtime != entry.mtime) {
                        files.push(file.path.clone());
                        if state == UpdateState::UpToDate {
                            state = UpdateState::Drifted;
                        }
                    }
                }
            }
        }

        // A manifest file count change is visible even without per-file tracking.
        if let Some(track) = tracked {
            if track.files.len() != manifest.files.len() && state == UpdateState::UpToDate {
                state = UpdateState::Outdated;
            }
        }

        TrackCheck {
            mod_name: manifest.name.clone(),
            state,
            tracked_checksum: tracked.map(|t| t.checksum.clone()),
            manifest_checksum: manifest.checksum.clone(),
            files,
            deep,
        }
    }

    /// Mod names recorded in the index.
    pub fn mod_names(&self) -> Vec<String> {
        self.mods.keys().cloned().collect()
    }
}

/// Deep-verify a single tracked file, used by the file detail view.
pub fn verify_file(path: &Path, checksum: &str) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    crate::checksum::md5_range(path, 0, meta.len())
        .map(|hash| hash.eq_ignore_ascii_case(checksum))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install::InstalledFile;

    fn manifest(name: &str, files: &[(&str, &[u8])]) -> ModManifest {
        // Build a real manifest through the generator so checksums are correct.
        let dir = tempfile::tempdir().unwrap();
        let mod_dir = dir.path().join(name);
        for (rel, data) in files {
            let path = mod_dir.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, data).unwrap();
        }
        crate::formats::generate_from_dir(name, &mod_dir).unwrap()
    }

    fn installed(manifest: &ModManifest, location: FileLocation) -> Vec<InstalledFile> {
        manifest
            .files
            .iter()
            .map(|file| InstalledFile {
                path: file.path.clone(),
                bytes: file.length,
                checksum: file.checksum.clone(),
                origin: match location {
                    FileLocation::Workshop => FileOrigin::Workshop,
                    FileLocation::Store => FileOrigin::Download,
                    FileLocation::Workspace => FileOrigin::Copy,
                },
                location,
                link: "hardlink".to_string(),
                source: None,
            })
            .collect()
    }

    fn workspace_with(manifest: &ModManifest) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join(&manifest.name);
        for file in &manifest.files {
            let path = target.join(&file.path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, vec![b'x'; file.length as usize]).unwrap();
        }
        dir
    }

    #[test]
    fn records_and_reloads_a_mod() {
        let manifest = manifest(
            "@demo",
            &[("mod.cpp", b"abc"), ("addons/a.pbo", b"payload")],
        );
        let workspace = workspace_with(&manifest);
        let mut index = RepoIndex::default();
        index.record(
            workspace.path(),
            Some("https://example/"),
            &manifest,
            "overlay",
            &installed(&manifest, FileLocation::Workspace),
        );
        index.save(workspace.path()).unwrap();

        let loaded = RepoIndex::load(workspace.path());
        assert_eq!(loaded.mods.len(), 1);
        let track = &loaded.mods["@demo"];
        assert_eq!(track.mode, "overlay");
        assert_eq!(track.files.len(), 2);
        assert_eq!(track.checksum, manifest.checksum);
        assert_eq!(track.files["mod.cpp"].bytes, 3);
        assert!(track.files["mod.cpp"].size > 0);

        let totals = loaded.totals("@demo");
        assert_eq!(totals.files, 2);
        assert_eq!(totals.logical_bytes, manifest.total_size());
        assert_eq!(totals.workspace_bytes, manifest.total_size());
        assert_eq!(totals.copied_bytes, manifest.total_size());
        assert_eq!(totals.saved_bytes(), 0);
    }

    #[test]
    fn totals_split_by_location() {
        let manifest = manifest("@demo", &[("a.bin", b"12345"), ("b.bin", b"678")]);
        let workspace = workspace_with(&manifest);
        let mut index = RepoIndex::default();
        let mut files = installed(&manifest, FileLocation::Workspace);
        files[0].location = FileLocation::Workshop;
        files[0].origin = FileOrigin::Workshop;
        files[1].location = FileLocation::Store;
        files[1].origin = FileOrigin::Download;
        index.record(workspace.path(), None, &manifest, "overlay", &files);

        let totals = index.totals("@demo");
        assert_eq!(totals.workshop_bytes, 5);
        assert_eq!(totals.shared_bytes, 3);
        assert_eq!(totals.workspace_bytes, 0);
        assert_eq!(totals.downloaded_bytes, 3);
        assert_eq!(totals.logical_bytes, 8);
        assert_eq!(totals.saved_bytes(), 8);
    }

    #[test]
    fn shallow_check_notices_a_missing_or_changed_file() {
        let manifest = manifest(
            "@demo",
            &[("mod.cpp", b"abc"), ("addons/a.pbo", b"payload")],
        );
        let workspace = workspace_with(&manifest);
        let target = workspace.path().join("@demo");
        let mut index = RepoIndex::default();
        index.record(
            workspace.path(),
            None,
            &manifest,
            "overlay",
            &installed(&manifest, FileLocation::Workspace),
        );

        let check = index.check(&target, &manifest, false, None);
        assert_eq!(check.state, UpdateState::UpToDate, "{check:?}");
        assert!(check.files.is_empty());

        fs::remove_file(target.join("addons/a.pbo")).unwrap();
        let check = index.check(&target, &manifest, false, None);
        assert_eq!(check.state, UpdateState::Missing);
        assert_eq!(check.files, vec!["addons/a.pbo".to_string()]);

        // A different repo checksum means the mod moved on.
        let mut other = manifest.clone();
        other.checksum = "DEADBEEF".into();
        assert_eq!(
            index.check(&target, &other, false, None).state,
            UpdateState::Outdated
        );
    }

    #[test]
    fn deep_check_finds_corrupt_bytes() {
        let manifest = manifest("@demo", &[("a.bin", b"12345")]);
        let workspace = workspace_with(&manifest);
        let target = workspace.path().join("@demo");
        let mut index = RepoIndex::default();
        index.record(
            workspace.path(),
            None,
            &manifest,
            "overlay",
            &installed(&manifest, FileLocation::Workspace),
        );
        // Same length, different bytes: only a deep check can see it.
        fs::write(target.join("a.bin"), b"12346").unwrap();
        let file_len = fs::metadata(target.join("a.bin")).unwrap().len();
        // Force the same length as recorded but different content.
        assert_eq!(file_len, 5);
        let shallow = index.check(&target, &manifest, false, None);
        let deep = index.check(&target, &manifest, true, None);
        assert_eq!(deep.state, UpdateState::Drifted);
        assert_eq!(deep.files, vec!["a.bin".to_string()]);
        assert!(shallow.state == UpdateState::UpToDate || shallow.state == UpdateState::Drifted);
    }

    #[test]
    fn untracked_mods_are_reported() {
        let manifest = manifest("@demo", &[("a.bin", b"1")]);
        let workspace = workspace_with(&manifest);
        let index = RepoIndex::default();
        let check = index.check(&workspace.path().join("@demo"), &manifest, false, None);
        assert_eq!(check.state, UpdateState::Untracked);
    }

    #[test]
    fn lock_entries_map_to_origins() {
        assert_eq!(
            FileOrigin::from_lock_entry("workshop:E:/x.pbo"),
            Some(FileOrigin::Workshop)
        );
        assert_eq!(
            FileOrigin::from_lock_entry("store:E:/s/x.pbo"),
            Some(FileOrigin::Store)
        );
        assert_eq!(
            FileOrigin::from_lock_entry("download:@m/x.pbo"),
            Some(FileOrigin::Download)
        );
        assert_eq!(FileOrigin::from_lock_entry("nonsense"), None);
    }
}
