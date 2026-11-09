//! Workspace statistics: how many bytes each repository really occupies, and
//! how many bytes were avoided by linking Workshop or shared-store content.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::Result;
use crate::store::{SharedStore, StoreStats};
use crate::workspace::{mod_dirs, read_lock, LockStats};

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoStats {
    pub repo: String,
    pub mods: usize,
    /// Size the mods would take if every file were a real copy.
    pub logical_bytes: u64,
    /// Bytes linked from the Steam Workshop cache.
    pub workshop_bytes: u64,
    /// Bytes linked from the shared store.
    pub store_bytes: u64,
    /// Bytes Nimby had to download into the shared store.
    pub downloaded_bytes: u64,
    /// Bytes physically copied (no link possible).
    pub copied_bytes: u64,
}

impl RepoStats {
    /// Bytes that already existed elsewhere (Steam cache or the shared store)
    /// and were linked instead of downloaded into the workspace.
    pub fn saved_bytes(&self) -> u64 {
        self.workshop_bytes + self.store_bytes
    }

    /// Bytes the workspace folder itself holds: the files Nimby had to place
    /// there because neither the Workshop cache nor the store had them.
    pub fn workspace_bytes(&self) -> u64 {
        self.logical_bytes.saturating_sub(self.saved_bytes())
    }

    /// Bytes this repository really occupies on disk: the store payload plus
    /// whatever the workspace holds. Workshop bytes belong to Steam, so they are
    /// reported separately instead of being counted here.
    ///
    /// Note that the store is shared, so it is counted once per file, not once
    /// per workspace.
    pub fn on_disk_bytes(&self) -> u64 {
        self.logical_bytes.saturating_sub(self.workshop_bytes)
    }
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModStats {
    pub mod_name: String,
    pub repo: String,
    pub mode: String,
    pub logical_bytes: u64,
    pub saved_bytes: u64,
    /// Bytes inside the workspace folder for this mod.
    pub workspace_bytes: u64,
    /// Bytes on disk including the shared-store payload.
    pub on_disk_bytes: u64,
    /// Bytes fetched from the repository.
    pub downloaded_bytes: u64,
    /// Bytes linked from the Steam Workshop cache.
    pub workshop_bytes: u64,
    /// Bytes held by the shared store.
    pub shared_bytes: u64,
    /// Bytes copied without a link.
    pub copied_bytes: u64,
    /// `true` when the per-file tracking index covers this mod.
    pub tracked: bool,
}

/// Aggregate of the tracking index across the install folders.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackSummary {
    pub mods: usize,
    pub files: usize,
    pub downloaded_bytes: u64,
    pub workshop_bytes: u64,
    pub shared_bytes: u64,
    pub workspace_bytes: u64,
    pub copied_bytes: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsReport {
    pub workspace: String,
    pub store: Option<StoreStats>,
    /// What the per-file tracking index knows, when it exists.
    pub tracking: TrackSummary,
    pub repos: Vec<RepoStats>,
    pub mods: Vec<ModStats>,
    pub totals: RepoStats,
}

impl StatsReport {
    pub fn saved_bytes(&self) -> u64 {
        self.totals.saved_bytes()
    }

    /// Bytes that exist only because of Nimby: store payload plus workspace copies.
    pub fn on_disk_bytes(&self) -> u64 {
        self.totals.on_disk_bytes()
    }

    /// Bytes the workspaces hold themselves, ignoring the store.
    pub fn workspace_bytes(&self) -> u64 {
        self.totals.workspace_bytes()
    }
}

fn repo_key(lock_repo: Option<&str>) -> String {
    match lock_repo {
        Some(repo) if !repo.trim().is_empty() => repo.trim().to_string(),
        _ => "(unknown repo)".to_string(),
    }
}

/// Aggregate every `.nimby` lock entry in a workspace.
pub fn collect(workspace: &Path, store: Option<&Path>) -> Result<StatsReport> {
    collect_many(&[workspace.to_path_buf()], store)
}

/// Aggregate several install folders. Each repository usually has its own, so
/// the Stats page can show them side by side.
pub fn collect_many(workspaces: &[PathBuf], store: Option<&Path>) -> Result<StatsReport> {
    let mut report = StatsReport {
        workspace: workspaces
            .first()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
        ..Default::default()
    };

    let mut seen: Vec<&PathBuf> = Vec::new();
    let mut dirs: Vec<PathBuf> = Vec::new();
    for workspace in workspaces {
        if seen.contains(&workspace) {
            continue;
        }
        seen.push(workspace);
        dirs.extend(mod_dirs(workspace).unwrap_or_default());
    }
    if workspaces.len() > 1 {
        report.workspace = workspaces
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(" · ");
    }

    for dir in dirs {
        let Some(lock) = read_lock(&dir) else {
            continue;
        };
        let repo = repo_key(lock.repo.as_deref());
        // The tracking index knows where every byte came from, so prefer it over
        // the aggregate counters in the lock (which cannot separate "downloaded
        // into the store" from "linked from the store").
        let folder = dir.parent().unwrap_or(Path::new("."));
        let track = crate::tracking::RepoIndex::load(folder)
            .mods
            .get(&lock.mod_name)
            .cloned();
        let stats = match &track {
            Some(track) => {
                let mut totals = crate::tracking::TrackTotals::default();
                for file in track.files.values() {
                    totals.add(file);
                }
                LockStats {
                    logical_bytes: totals.logical_bytes,
                    workshop_bytes: totals.workshop_bytes,
                    store_bytes: totals.shared_bytes,
                    downloaded_bytes: totals.downloaded_bytes,
                    copied_bytes: totals.copied_bytes,
                }
            }
            None => lock.stats,
        };
        let tracked = track.is_some();
        if tracked {
            report.tracking.mods += 1;
            report.tracking.files += track.as_ref().map(|t| t.files.len()).unwrap_or(0);
            report.tracking.downloaded_bytes += stats.downloaded_bytes;
            report.tracking.workshop_bytes += stats.workshop_bytes;
            report.tracking.shared_bytes += stats.store_bytes;
            report.tracking.copied_bytes += stats.copied_bytes;
            report.tracking.workspace_bytes += stats
                .logical_bytes
                .saturating_sub(stats.workshop_bytes + stats.store_bytes);
        }

        let entry = match report.repos.iter_mut().find(|r| r.repo == repo) {
            Some(entry) => entry,
            None => {
                report.repos.push(RepoStats {
                    repo: repo.clone(),
                    ..Default::default()
                });
                report.repos.last_mut().expect("just pushed")
            }
        };
        entry.mods += 1;
        entry.logical_bytes += stats.logical_bytes;
        entry.workshop_bytes += stats.workshop_bytes;
        entry.store_bytes += stats.store_bytes;
        entry.downloaded_bytes += stats.downloaded_bytes;
        entry.copied_bytes += stats.copied_bytes;

        let saved = stats.workshop_bytes + stats.store_bytes;
        let logical = stats.logical_bytes;
        report.mods.push(ModStats {
            mod_name: lock.mod_name.clone(),
            repo: repo.clone(),
            mode: lock.mode.clone(),
            logical_bytes: logical,
            saved_bytes: saved,
            workspace_bytes: logical.saturating_sub(saved),
            on_disk_bytes: logical.saturating_sub(stats.workshop_bytes),
            downloaded_bytes: stats.downloaded_bytes,
            workshop_bytes: stats.workshop_bytes,
            shared_bytes: stats.store_bytes,
            copied_bytes: stats.copied_bytes,
            tracked,
        });
    }

    report
        .repos
        .sort_by_key(|repo| std::cmp::Reverse(repo.saved_bytes()));
    report
        .mods
        .sort_by_key(|m| std::cmp::Reverse(m.saved_bytes));

    for repo in &report.repos {
        report.totals.mods += repo.mods;
        report.totals.logical_bytes += repo.logical_bytes;
        report.totals.workshop_bytes += repo.workshop_bytes;
        report.totals.store_bytes += repo.store_bytes;
        report.totals.downloaded_bytes += repo.downloaded_bytes;
        report.totals.copied_bytes += repo.copied_bytes;
    }
    report.totals.repo = "(total)".to_string();

    if let Some(root) = store {
        report.store = SharedStore::new(root).stats().ok();
    }

    Ok(report)
}

/// Convenience for the UI: human readable size.
pub fn human(bytes: u64) -> String {
    crate::util::format_size(bytes)
}

/// Absolute path helper used by the CLI/GUI to display where a repo lives.
pub fn workspace_of(config: &crate::config::AppConfig) -> PathBuf {
    config.resolved_workspace()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{write_lock_full, LockStats};
    use std::collections::BTreeMap;
    use std::fs;
    fn seed(workspace: &Path, mod_name: &str, repo: &str, stats: LockStats) {
        let target = workspace.join(mod_name);
        fs::create_dir_all(&target).unwrap();
        let manifest = crate::formats::ModManifest::parse(
            "\u{feff}{\"Name\":\"@x\",\"Checksum\":\"AB\",\"Files\":[]}".as_bytes(),
        )
        .unwrap();
        write_lock_full(
            &target,
            &manifest,
            Some(repo),
            "overlay",
            BTreeMap::new(),
            stats,
        )
        .unwrap();
    }

    #[test]
    fn aggregates_per_repo() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();

        seed(
            &ws,
            "@a",
            "https://one/",
            LockStats {
                logical_bytes: 1000,
                workshop_bytes: 600,
                store_bytes: 200,
                downloaded_bytes: 200,
                copied_bytes: 0,
            },
        );
        seed(
            &ws,
            "@b",
            "https://one/",
            LockStats {
                logical_bytes: 500,
                workshop_bytes: 0,
                store_bytes: 500,
                downloaded_bytes: 500,
                copied_bytes: 0,
            },
        );
        seed(
            &ws,
            "@c",
            "https://two/",
            LockStats {
                logical_bytes: 100,
                workshop_bytes: 0,
                store_bytes: 0,
                downloaded_bytes: 100,
                copied_bytes: 100,
            },
        );

        let report = collect(&ws, None).unwrap();
        assert_eq!(report.repos.len(), 2);
        assert_eq!(report.totals.mods, 3);
        assert_eq!(report.totals.saved_bytes(), 1300);
        // The workspace itself holds only the bytes nothing else had.
        assert_eq!(report.workspace_bytes(), 300);
        // Store payload counts as real disk usage; Workshop bytes belong to Steam.
        assert_eq!(report.on_disk_bytes(), 1000);
        // The biggest saver keeps only its store copy on disk: 1000 logical
        // minus the 600 bytes Steam already had.
        assert_eq!(report.mods[0].on_disk_bytes, 400);

        let first = &report.repos[0];
        assert_eq!(first.repo, "https://one/");
        assert_eq!(first.mods, 2);
        assert_eq!(first.saved_bytes(), 1300);
    }

    #[test]
    fn tracking_index_wins_over_lock_counters() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        seed(
            ws,
            "@a",
            "https://one/",
            LockStats {
                logical_bytes: 1000,
                workshop_bytes: 0,
                store_bytes: 1000,
                downloaded_bytes: 1000,
                copied_bytes: 0,
            },
        );

        // A tracking entry says: 400 of those bytes are Workshop-linked and the
        // rest came from the store.
        // The seeded lock is named "@x", so the index must use the same key.
        let manifest = crate::formats::ModManifest::parse(
            "\u{feff}{\"Name\":\"@x\",\"Checksum\":\"AB\",\"Files\":[]}".as_bytes(),
        )
        .unwrap();
        let mut index = crate::tracking::RepoIndex::default();
        let files = vec![
            crate::install::InstalledFile {
                path: "a.bin".into(),
                bytes: 400,
                checksum: "1".into(),
                origin: crate::tracking::FileOrigin::Workshop,
                location: crate::tracking::FileLocation::Workshop,
                link: "hardlink".into(),
                source: None,
            },
            crate::install::InstalledFile {
                path: "b.bin".into(),
                bytes: 600,
                checksum: "2".into(),
                origin: crate::tracking::FileOrigin::Download,
                location: crate::tracking::FileLocation::Store,
                link: "hardlink".into(),
                source: None,
            },
        ];
        index.record(ws, Some("https://one/"), &manifest, "overlay", &files);
        index.save(ws).unwrap();

        let report = collect(ws, None).unwrap();
        let repo = &report.repos[0];
        assert_eq!(repo.workshop_bytes, 400);
        assert_eq!(repo.store_bytes, 600);
        assert_eq!(repo.downloaded_bytes, 600);
        assert_eq!(repo.on_disk_bytes(), 600);
        assert!(report.mods[0].tracked);
        assert_eq!(report.tracking.mods, 1);
        assert_eq!(report.tracking.files, 2);
        assert_eq!(report.tracking.workshop_bytes, 400);
        assert_eq!(report.tracking.shared_bytes, 600);
    }

    #[test]
    fn collects_from_several_repository_folders() {
        let one = tempfile::tempdir().unwrap();
        let two = tempfile::tempdir().unwrap();
        seed(
            one.path(),
            "@a",
            "https://one/",
            LockStats {
                logical_bytes: 100,
                ..Default::default()
            },
        );
        seed(
            two.path(),
            "@b",
            "https://two/",
            LockStats {
                logical_bytes: 40,
                ..Default::default()
            },
        );

        let report =
            collect_many(&[one.path().to_path_buf(), two.path().to_path_buf()], None).unwrap();
        assert_eq!(report.repos.len(), 2);
        assert_eq!(report.totals.mods, 2);
        assert_eq!(report.totals.logical_bytes, 140);

        // The same folder twice must not double count.
        let report =
            collect_many(&[one.path().to_path_buf(), one.path().to_path_buf()], None).unwrap();
        assert_eq!(report.totals.mods, 1);
    }

    #[test]
    fn mods_without_repo_are_grouped() {
        let dir = tempfile::tempdir().unwrap();
        seed(
            dir.path(),
            "@x",
            "",
            LockStats {
                logical_bytes: 10,
                ..Default::default()
            },
        );
        let report = collect(dir.path(), None).unwrap();
        assert_eq!(report.repos[0].repo, "(unknown repo)");
    }
}
