//! Persisted application configuration: paths, link mode, launch parameters
//! and the list of managed repositories.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::linker::LinkMode;
use crate::params::LaunchParam;
use crate::util::strip_bom;

/// A repository the user has added.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RepoEntry {
    pub url: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    /// Last time this repo was inspected, as a unix timestamp.
    #[serde(default)]
    pub last_checked: Option<i64>,
    /// Subfolder of the workspace root this repository installs into. Every
    /// repository is isolated in its own folder; `None` means "derive one from
    /// the name".
    #[serde(default)]
    pub folder: Option<String>,
    /// Legacy absolute install folder, migrated into `folder` on load so old
    /// configs keep working.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
}

impl RepoEntry {
    pub fn display_name(&self) -> &str {
        if self.name.is_empty() {
            &self.url
        } else {
            &self.name
        }
    }

    /// Install folder of this repo: `<workspace root>/<folder>`.
    pub fn workspace_under(&self, root: &Path) -> PathBuf {
        root.join(folder_for(self))
    }

    /// `true` when no folder was chosen explicitly.
    pub fn has_auto_folder(&self) -> bool {
        self.folder
            .as_deref()
            .map(|f| f.trim().is_empty())
            .unwrap_or(true)
    }
}

/// Turn anything a user typed into a safe single folder name.
pub fn sanitize_folder(raw: &str) -> String {
    let mut out = String::new();
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '.' {
            out.push(ch);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches(|c| c == '-' || c == '.')
        .trim()
        .to_string()
}

/// Folder for a repo: the explicit choice, else its name, else its URL.
pub fn folder_for(repo: &RepoEntry) -> String {
    if let Some(folder) = &repo.folder {
        let clean = sanitize_folder(folder);
        if !clean.is_empty() {
            return clean;
        }
    }
    let from_name = sanitize_folder(repo.display_name());
    if !from_name.is_empty() && !repo.name.is_empty() {
        return from_name;
    }
    // Fall back to the last path segment of the URL, then the host.
    let trimmed = repo.url.trim_end_matches('/');
    if let Some(segment) = trimmed
        .rsplit('/')
        .find(|s| !s.is_empty() && !s.contains(':'))
    {
        let clean = sanitize_folder(segment);
        if !clean.is_empty() {
            return clean;
        }
    }
    let host = trimmed
        .split("://")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or_default();
    let clean = sanitize_folder(host);
    if clean.is_empty() {
        "repo".to_string()
    } else {
        clean
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppConfig {
    /// Arma 3 install folder. `None` means "autodetect on use".
    pub arma_path: Option<PathBuf>,
    /// Folder used for scratch data (partial downloads, temp files).
    pub temp_path: Option<PathBuf>,
    /// Shared, content-addressed mod store. When set, downloaded files land
    /// here once and are hardlinked into every workspace that needs them.
    pub shared_path: Option<PathBuf>,
    /// Where mods are installed and launched from.
    pub workspace_path: PathBuf,
    pub link_mode: String,
    pub parallel_downloads: u32,
    /// How many mods are installed at the same time. Each mod keeps its own
    /// file-level parallelism, so the two values multiply.
    pub parallel_mods: u32,
    pub battleye: bool,
    /// Extra launch parameters, always applied.
    pub params: Vec<LaunchParam>,
    pub repos: Vec<RepoEntry>,
    /// URL of the repo the UI works with by default.
    pub active_repo: Option<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            arma_path: None,
            temp_path: None,
            shared_path: None,
            workspace_path: default_workspace(),
            link_mode: "auto".to_string(),
            parallel_downloads: 8,
            parallel_mods: 2,
            battleye: false,
            params: Vec::new(),
            repos: Vec::new(),
            active_repo: None,
        }
    }
}

pub fn config_dir() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("Nimby")
}

pub fn default_workspace() -> PathBuf {
    let base = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("Nimby").join("mods")
}

/// Default location of the Swifty client's settings, for `migrate`.
pub fn swifty_settings_path() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("Swifty").join("Settings.json")
}

/// Where [`config_path`] takes the configuration file from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigLocation {
    /// `NIMBY_CONFIG` points at an explicit file.
    Environment,
    /// The executable sits in a writable folder, so its `data` folder is used.
    Portable,
    /// The regular per-user configuration folder.
    UserProfile,
}

impl ConfigLocation {
    /// Stable, machine readable name for reports and the CLI snapshot.
    pub fn as_str(self) -> &'static str {
        match self {
            ConfigLocation::Environment => "environment",
            ConfigLocation::Portable => "portable",
            ConfigLocation::UserProfile => "user-profile",
        }
    }
}

fn env_config_path() -> Option<PathBuf> {
    // `NIMBY_CONFIG` lets portable installs and tests use another file without
    // touching the real user configuration.
    std::env::var_os("NIMBY_CONFIG")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

/// `<exe dir>\data`, when that folder exists or can be created and written to.
/// The portable release keeps the GUI in the root folder and the CLI in `bin\`,
/// so a CLI launched from there shares the same `data` folder.
fn portable_dir_for(exe_dir: &Path) -> Option<PathBuf> {
    let in_bin = exe_dir
        .file_name()
        .map(|name| name.eq_ignore_ascii_case("bin"))
        .unwrap_or(false);
    let candidates = if in_bin {
        match exe_dir.parent() {
            Some(parent) => vec![parent.join("data"), exe_dir.join("data")],
            None => vec![exe_dir.join("data")],
        }
    } else {
        vec![exe_dir.join("data")]
    };
    candidates.into_iter().find(|dir| data_dir_is_writable(dir))
}

/// Create `dir` and write a marker into it: only a real write tells whether the
/// install folder is read-only, as with a build under Program Files.
fn data_dir_is_writable(dir: &Path) -> bool {
    if fs::create_dir_all(dir).is_err() {
        return false;
    }
    let marker = dir.join(format!(".nimby-write-test-{}", std::process::id()));
    let writable = fs::write(&marker, b"").is_ok();
    let _ = fs::remove_file(&marker);
    writable
}

/// Location and file of the configuration, resolved on every call.
fn resolved_config() -> (ConfigLocation, PathBuf) {
    if let Some(path) = env_config_path() {
        return (ConfigLocation::Environment, path);
    }
    let portable = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .and_then(|dir| portable_dir_for(&dir));
    if let Some(dir) = portable {
        return (ConfigLocation::Portable, dir.join("config.json"));
    }
    (
        ConfigLocation::UserProfile,
        config_dir().join("config.json"),
    )
}

/// Where the configuration file comes from.
pub fn config_location() -> ConfigLocation {
    resolved_config().0
}

pub fn config_path() -> PathBuf {
    resolved_config().1
}

/// Folder that keeps data belonging to the configuration, so a portable
/// install carries its cache in `<exe>\data` as well.
pub fn data_dir() -> PathBuf {
    config_path()
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(config_dir)
}

impl AppConfig {
    pub fn load_from(path: &Path) -> Result<Self> {
        if !path.is_file() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(path)?;
        let config: AppConfig = serde_json::from_str(strip_bom(&text))?;
        Ok(config.normalized())
    }

    pub fn load() -> Result<Self> {
        Self::load_from(&config_path())
    }

    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(self)?;
        fs::write(path, text)?;
        Ok(())
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&config_path())
    }

    /// Fill in values that are missing or invalid.
    fn normalized(mut self) -> Self {
        if self.workspace_path.as_os_str().is_empty() {
            self.workspace_path = default_workspace();
        }
        if self.link_mode.is_empty() || LinkMode::parse(&self.link_mode).is_none() {
            self.link_mode = "auto".to_string();
        }
        if self.parallel_downloads == 0 {
            self.parallel_downloads = 8;
        }
        if self.parallel_mods == 0 {
            self.parallel_mods = 2;
        }
        // Drop a stale active repo pointer.
        if let Some(active) = &self.active_repo {
            if !self.repos.iter().any(|r| &r.url == active) {
                self.active_repo = self.repos.first().map(|r| r.url.clone());
            }
        }
        // Repo URLs are compared everywhere, so store them canonical.
        let active = self.active_repo.clone();
        for repo in &mut self.repos {
            repo.url = normalize_url(&repo.url);
            // Old configs stored an absolute install folder; keep the last
            // component as the subfolder so nothing is lost.
            let legacy = repo.path.take();
            let candidate = repo
                .folder
                .clone()
                .filter(|f| !f.trim().is_empty())
                .or_else(|| {
                    legacy
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .map(|n| n.to_string_lossy().to_string())
                });
            repo.folder = candidate
                .map(|f| sanitize_folder(&f))
                .filter(|f| !f.is_empty())
                .or_else(|| Some(folder_for(repo)));
        }
        self.active_repo = active.map(|url| normalize_url(&url));

        // Two repositories must never share a folder.
        let mut used: Vec<String> = Vec::new();
        for repo in &mut self.repos {
            let base = repo.folder.clone().unwrap_or_else(|| "repo".to_string());
            let mut candidate = base.clone();
            let mut suffix = 2;
            while used.iter().any(|u| u.eq_ignore_ascii_case(&candidate)) {
                candidate = format!("{base}-{suffix}");
                suffix += 1;
            }
            used.push(candidate.clone());
            repo.folder = Some(candidate);
        }

        self.params.retain(|p| !p.key.trim().is_empty());
        self
    }

    pub fn link_mode(&self) -> LinkMode {
        LinkMode::parse(&self.link_mode).unwrap_or(LinkMode::Auto)
    }

    /// Arma install folder: configured value, else autodetected.
    pub fn resolved_arma(&self) -> Option<PathBuf> {
        if let Some(path) = &self.arma_path {
            if looks_like_arma(path) {
                return Some(path.clone());
            }
        }
        crate::steam::find_arma3()
    }

    /// Scratch folder for partial downloads.
    pub fn resolved_temp(&self) -> PathBuf {
        if let Some(path) = &self.temp_path {
            return path.clone();
        }
        std::env::temp_dir().join("nimby")
    }

    pub fn resolved_workspace(&self) -> PathBuf {
        self.workspace_path.clone()
    }

    /// The workspace root that holds one subfolder per repository.
    pub fn workspace_root(&self) -> PathBuf {
        self.workspace_path.clone()
    }

    /// Install folder for one repository: `<root>/<folder>`, always isolated.
    pub fn workspace_for(&self, url: &str) -> PathBuf {
        let key = normalize_url(url);
        self.repos
            .iter()
            .find(|repo| repo.url == key)
            .map(|repo| repo.workspace_under(&self.workspace_path))
            .unwrap_or_else(|| self.workspace_path.join("repo"))
    }

    /// Install folder of the selected repository. Everything the GUI and the
    /// CLI show is about this folder, so a repo and its path stay paired.
    pub fn active_workspace(&self) -> PathBuf {
        self.active_repo()
            .map(|repo| repo.workspace_under(&self.workspace_path))
            .unwrap_or_else(|| self.workspace_path.clone())
    }

    /// Every distinct install folder, paired with the repo that owns it.
    pub fn workspaces(&self) -> Vec<(Option<String>, PathBuf)> {
        let mut out: Vec<(Option<String>, PathBuf)> = Vec::new();
        for repo in &self.repos {
            let path = repo.workspace_under(&self.workspace_path);
            if let Some(existing) = out.iter_mut().find(|(_, p)| *p == path) {
                if existing.0.is_none() {
                    existing.0 = Some(repo.url.clone());
                }
            } else {
                out.push((Some(repo.url.clone()), path));
            }
        }
        if out.is_empty() {
            out.push((None, self.workspace_path.clone()));
        }
        out
    }

    /// Shared store, only when the user enabled one.
    pub fn resolved_shared(&self) -> Option<PathBuf> {
        self.shared_path
            .clone()
            .filter(|p| !p.as_os_str().is_empty())
    }

    pub fn active_repo(&self) -> Option<&RepoEntry> {
        let active = self.active_repo.as_deref()?;
        self.repos.iter().find(|r| r.url == active)
    }

    /// Add a repo or update its name/credentials/folder. Returns the entry index.
    pub fn upsert_repo(&mut self, entry: RepoEntry) -> usize {
        let url = normalize_url(&entry.url);
        if let Some(index) = self.repos.iter().position(|r| r.url == url) {
            let existing = &mut self.repos[index];
            if !entry.name.is_empty() {
                existing.name = entry.name;
            }
            if !entry.username.is_empty() {
                existing.username = entry.username;
            }
            if !entry.password.is_empty() {
                existing.password = entry.password;
            }
            if entry.folder.is_some() {
                existing.folder = entry.folder;
            }
            if existing
                .folder
                .as_deref()
                .map(str::trim)
                .unwrap_or("")
                .is_empty()
            {
                existing.folder = Some(folder_for(existing));
            }
            if self.active_repo.is_none() {
                self.active_repo = Some(url);
            }
            return index;
        }
        let mut entry = entry;
        entry.url = url.clone();
        if entry
            .folder
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty()
        {
            entry.folder = Some(folder_for(&entry));
        }
        self.repos.push(entry);
        if self.active_repo.is_none() {
            self.active_repo = Some(url);
        }
        self.repos.len() - 1
    }

    pub fn remove_repo(&mut self, url: &str) -> bool {
        let url = normalize_url(url);
        let before = self.repos.len();
        self.repos.retain(|r| r.url != url);
        let removed = self.repos.len() != before;
        if removed && self.active_repo.as_deref() == Some(url.as_str()) {
            self.active_repo = self.repos.first().map(|r| r.url.clone());
        }
        removed
    }

    pub fn select_repo(&mut self, url: &str) -> bool {
        let url = normalize_url(url);
        if self.repos.iter().any(|r| r.url == url) {
            self.active_repo = Some(url);
            true
        } else {
            false
        }
    }

    /// Point a repo at another subfolder (`None` restores the derived name).
    pub fn set_repo_folder(&mut self, url: &str, folder: Option<String>) -> bool {
        let url = normalize_url(url);
        match self.repos.iter_mut().find(|r| r.url == url) {
            Some(repo) => {
                repo.folder = folder
                    .map(|f| sanitize_folder(&f))
                    .filter(|f| !f.is_empty());
                if repo.folder.is_none() {
                    repo.folder = Some(folder_for(repo));
                }
                true
            }
            None => false,
        }
    }
}

/// Trailing slash, no surrounding whitespace.
pub fn normalize_url(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.is_empty() || trimmed.ends_with('/') {
        trimmed.to_string()
    } else {
        format!("{trimmed}/")
    }
}

pub fn looks_like_arma(path: &Path) -> bool {
    path.join("arma3_x64.exe").is_file()
        || path.join("arma3.exe").is_file()
        || path.join("arma3server_x64.exe").is_file()
}

pub fn ensure_dir(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() {
        return Err(Error::msg("empty directory path"));
    }
    fs::create_dir_all(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compare paths regardless of the separator the platform uses.
    fn shown(path: &Path) -> String {
        path.to_string_lossy().replace('\\', "/")
    }

    #[test]
    fn round_trip_and_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");

        let mut config = AppConfig {
            shared_path: Some(dir.path().join("shared")),
            ..AppConfig::default()
        };
        config.upsert_repo(RepoEntry {
            url: "https://example.com/repo".into(),
            name: "Example".into(),
            ..Default::default()
        });
        config.select_repo("https://example.com/repo");
        config.params.push(LaunchParam::parse("-name=Fred"));
        config.save_to(&path).unwrap();

        let loaded = AppConfig::load_from(&path).unwrap();
        assert_eq!(loaded.repos.len(), 1);
        assert_eq!(loaded.repos[0].url, "https://example.com/repo/");
        assert_eq!(
            loaded.active_repo.as_deref(),
            Some("https://example.com/repo/")
        );
        assert_eq!(loaded.params[0].key, "-name");
        assert_eq!(loaded.params[0].value.as_deref(), Some("Fred"));
        assert_eq!(loaded.link_mode, "auto");
    }

    #[test]
    fn every_repo_gets_its_own_folder_under_the_root() {
        let mut config = AppConfig {
            workspace_path: PathBuf::from("D:/mods"),
            ..AppConfig::default()
        };
        config.upsert_repo(RepoEntry {
            url: "https://one/repo".into(),
            name: "One".into(),
            ..Default::default()
        });
        config.upsert_repo(RepoEntry {
            url: "https://two/repo".into(),
            name: "Two Community".into(),
            ..Default::default()
        });

        // Derived from the display name, isolated under the root.
        assert_eq!(
            shown(&config.workspace_for("https://one/repo/")),
            "D:/mods/One"
        );
        assert_eq!(
            shown(&config.workspace_for("https://two/repo/")),
            "D:/mods/Two-Community"
        );

        config.select_repo("https://one/repo");
        assert_eq!(shown(&config.active_workspace()), "D:/mods/One");

        let workspaces = config.workspaces();
        assert_eq!(workspaces.len(), 2);
        assert_eq!(shown(&workspaces[0].1), "D:/mods/One");
        assert_eq!(shown(&workspaces[1].1), "D:/mods/Two-Community");

        // The folder is renameable but always stays below the root.
        assert!(config.set_repo_folder("https://two/repo", Some("my-community".into())));
        assert_eq!(
            shown(&config.workspace_for("https://two/repo/")),
            "D:/mods/my-community"
        );
        assert_eq!(config.repos[1].name, "Two Community");

        // Resetting restores the derived name.
        assert!(config.set_repo_folder("https://two/repo", None));
        assert_eq!(
            shown(&config.workspace_for("https://two/repo/")),
            "D:/mods/Two-Community"
        );
        assert!(!config.set_repo_folder("https://nope/", None));

        // A path-like folder name cannot escape the root.
        assert!(config.set_repo_folder("https://one/repo", Some("../escape".into())));
        let path = config.workspace_for("https://one/repo/");
        assert!(path.starts_with(config.workspace_root()), "{path:?}");
    }

    #[test]
    fn folders_never_collide() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(
            &path,
            r#"{ "workspacePath": "D:/mods", "repos": [
                { "url": "https://a/repo", "name": "Same" },
                { "url": "https://b/repo", "name": "Same" }
            ] }"#,
        )
        .unwrap();
        let config = AppConfig::load_from(&path).unwrap();
        assert_eq!(config.repos[0].folder.as_deref(), Some("Same"));
        assert_eq!(config.repos[1].folder.as_deref(), Some("Same-2"));
        assert_ne!(
            config.workspace_for("https://a/repo/"),
            config.workspace_for("https://b/repo/")
        );
    }

    #[test]
    fn legacy_absolute_paths_become_subfolders() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(
            &path,
            r#"{ "workspacePath": "E:/Nimby", "repos": [
                { "url": "https://cdn.synixe.contractors/repo/main/", "path": "E:/Nimby/Synixe2" }
            ] }"#,
        )
        .unwrap();
        let config = AppConfig::load_from(&path).unwrap();
        assert_eq!(config.repos[0].folder.as_deref(), Some("Synixe2"));
        assert_eq!(
            shown(&config.workspace_for("https://cdn.synixe.contractors/repo/main/")),
            "E:/Nimby/Synixe2"
        );
    }

    #[test]
    fn urls_are_normalized_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(
            &path,
            r#"{ "repos": [{ "url": "https://example.com/repo" }], "activeRepo": "https://example.com/repo" }"#,
        )
        .unwrap();
        let config = AppConfig::load_from(&path).unwrap();
        assert_eq!(config.repos[0].url, "https://example.com/repo/");
        assert_eq!(
            config.active_repo.as_deref(),
            Some("https://example.com/repo/")
        );
    }

    #[test]
    fn removing_active_repo_moves_selection() {
        let mut config = AppConfig::default();
        config.upsert_repo(RepoEntry {
            url: "https://a/".into(),
            ..Default::default()
        });
        config.upsert_repo(RepoEntry {
            url: "https://b/".into(),
            ..Default::default()
        });
        config.select_repo("https://a/");
        assert!(config.remove_repo("https://a/"));
        assert_eq!(config.active_repo.as_deref(), Some("https://b/"));
        assert!(!config.remove_repo("https://missing/"));
    }

    #[test]
    fn empty_config_has_usable_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = AppConfig::load_from(&dir.path().join("missing.json")).unwrap();
        assert!(!loaded.workspace_path.as_os_str().is_empty());
        assert_eq!(loaded.link_mode(), LinkMode::Auto);
        assert!(loaded.resolved_shared().is_none());
    }

    #[test]
    fn portable_dir_used_when_executable_folder_is_writable() {
        let dir = tempfile::tempdir().unwrap();
        let portable = portable_dir_for(dir.path()).expect("writable folder is portable");
        assert_eq!(portable, dir.path().join("data"));
        assert!(portable.is_dir());
        // The write probe must not leave its marker behind.
        assert_eq!(fs::read_dir(&portable).unwrap().count(), 0);
    }

    #[test]
    fn portable_dir_skipped_when_data_is_a_file() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("data"), b"not a folder").unwrap();
        assert!(portable_dir_for(dir.path()).is_none());
    }

    #[test]
    fn cli_in_bin_shares_the_data_folder_of_its_parent() {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        let portable = portable_dir_for(&bin).expect("the install folder is writable");
        assert_eq!(portable, root.path().join("data"));
        // The CLI must not create a second configuration inside bin\.
        assert!(!bin.join("data").exists());
    }

    #[test]
    fn cli_in_bin_falls_back_to_its_own_data_folder_when_the_parent_is_read_only() {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        // A file where the parent's data folder would go makes it unusable.
        fs::write(root.path().join("data"), b"blocked").unwrap();
        let portable = portable_dir_for(&bin).expect("bin itself stays writable");
        assert_eq!(portable, bin.join("data"));
    }

    #[test]
    fn config_location_reports_environment_when_env_var_is_set() {
        // `NIMBY_CONFIG` is process wide, so this test restores it right away.
        let previous = std::env::var_os("NIMBY_CONFIG");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("custom.json");
        std::env::set_var("NIMBY_CONFIG", &path);
        let location = config_location();
        let resolved = config_path();
        match previous {
            Some(value) => std::env::set_var("NIMBY_CONFIG", value),
            None => std::env::remove_var("NIMBY_CONFIG"),
        }

        assert_eq!(location, ConfigLocation::Environment);
        assert_eq!(resolved, path);
    }
}
