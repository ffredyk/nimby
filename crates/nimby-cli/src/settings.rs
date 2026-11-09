use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{json, Value};

use nimby_core::config::{self, AppConfig, RepoEntry};
use nimby_core::params::{LaunchParam, ParamKind, KNOWN_PARAMS};
use nimby_core::store::SharedStore;
use nimby_core::LinkMode;

use crate::RepoAction;

pub fn load_config() -> Result<AppConfig> {
    AppConfig::load().context("reading Nimby config")
}

pub fn param_kind_name(kind: ParamKind) -> &'static str {
    match kind {
        ParamKind::Flag => "flag",
        ParamKind::Text => "text",
        ParamKind::Number => "number",
        ParamKind::Path => "path",
        ParamKind::Choice => "choice",
    }
}

/// Everything the UI needs in one payload.
pub fn config_snapshot(config: &AppConfig) -> Value {
    let detected = nimby_core::steam::find_arma3();
    let effective_arma = config.resolved_arma();
    let shared = config.resolved_shared();
    let store_stats = shared
        .as_ref()
        .and_then(|path| SharedStore::new(path).stats().ok());

    let params: Vec<Value> = config
        .params
        .iter()
        .map(|param| {
            let spec = nimby_core::params::find_spec(&param.key);
            json!({
                "key": param.key,
                "value": param.value,
                "enabled": param.enabled,
                "display": param.display(),
                "valid": param.is_valid(),
                "known": spec.is_some(),
                "label": spec.map(|s| s.label),
                "kind": spec.map(|s| param_kind_name(s.kind)),
                "choices": spec.map(|s| s.choices.to_vec()),
                "description": spec.map(|s| s.description),
                "management": if spec.map(|s| s.managed_by_nimby).unwrap_or(false) { "auto" } else { "user" },
            })
        })
        .collect();

    let repos: Vec<Value> = config
        .repos
        .iter()
        .map(|repo| {
            json!({
                "url": repo.url,
                "name": repo.display_name(),
                "customName": repo.name,
                "username": repo.username,
                "hasPassword": !repo.password.is_empty(),
                "active": config.active_repo.as_deref() == Some(repo.url.as_str()),
                // The folder name under the workspace root, and the full path.
                "folder": nimby_core::config::folder_for(repo),
                "path": repo.workspace_under(&config.workspace_path).display().to_string(),
                "autoFolder": repo.has_auto_folder(),
            })
        })
        .collect();

    let known: Vec<Value> = KNOWN_PARAMS
        .iter()
        .map(|spec| {
            json!({
                "key": spec.key,
                "label": spec.label,
                "kind": param_kind_name(spec.kind),
                "choices": spec.choices,
                "alwaysQuote": spec.always_quote,
                "description": spec.description,
                "managedByNimby": spec.managed_by_nimby,
            })
        })
        .collect();

    json!({
        "configPath": config::config_path().display().to_string(),
        "configDir": config::config_dir().display().to_string(),
        "armaPath": config.arma_path.as_ref().map(|p| p.display().to_string()),
        "detectedArma": detected.as_ref().map(|p| p.display().to_string()),
        "effectiveArma": effective_arma.as_ref().map(|p| p.display().to_string()),
        "armaValid": effective_arma.as_ref().map(|p| config::looks_like_arma(p)).unwrap_or(false),
        "tempPath": config.temp_path.as_ref().map(|p| p.display().to_string()),
        "effectiveTemp": config.resolved_temp().display().to_string(),
        "sharedPath": shared.as_ref().map(|p| p.display().to_string()),
        "sharedStats": store_stats,
        "workspacePath": config.resolved_workspace().display().to_string(),
        "activeWorkspace": config.active_workspace().display().to_string(),
        "linkMode": config.link_mode,
        "linkModes": ["auto", "dir-link", "overlay", "copy", "passthrough"],
        "parallelDownloads": config.parallel_downloads,
        "parallelMods": config.parallel_mods,
        "battleye": config.battleye,
        "params": params,
        "repos": repos,
        "activeRepo": config.active_repo,
        "knownParams": known,
    })
}

fn print_snapshot(config: &AppConfig, json_out: bool) -> Result<()> {
    let snapshot = config_snapshot(config);
    if json_out {
        println!("{}", serde_json::to_string_pretty(&snapshot)?);
        return Ok(());
    }

    let text = |key: &str| -> String {
        snapshot
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string()
    };
    let opt = |key: &str| -> String {
        snapshot
            .get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "(not set)".to_string())
    };

    println!("config    {}", text("configPath"));
    println!("arma      {}", opt("effectiveArma"));
    println!("temp      {}", text("effectiveTemp"));
    println!("shared    {}", opt("sharedPath"));
    println!("workspace {}", text("workspacePath"));
    println!("active    {}", text("activeWorkspace"));
    println!(
        "mode      {} · {} parallel downloads · battleye {}",
        text("linkMode"),
        snapshot
            .get("parallelDownloads")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        snapshot
            .get("battleye")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    );
    if let Some(repos) = snapshot.get("repos").and_then(|v| v.as_array()) {
        println!("repos     {}", repos.len());
        for repo in repos {
            println!(
                "  {}{}  {}",
                if repo
                    .get("active")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    "* "
                } else {
                    "  "
                },
                repo.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                repo.get("url").and_then(|v| v.as_str()).unwrap_or(""),
            );
        }
    }
    if let Some(params) = snapshot.get("params").and_then(|v| v.as_array()) {
        for param in params {
            println!(
                "  param {}",
                param.get("display").and_then(|v| v.as_str()).unwrap_or("")
            );
        }
    }
    Ok(())
}

pub fn cmd_config_show(json_out: bool) -> Result<()> {
    let config = load_config()?;
    print_snapshot(&config, json_out)
}

pub struct SetArgs {
    pub arma: Option<PathBuf>,
    pub clear_arma: bool,
    pub temp: Option<PathBuf>,
    pub clear_temp: bool,
    pub shared: Option<PathBuf>,
    pub clear_shared: bool,
    pub workspace: Option<PathBuf>,
    pub link_mode: Option<String>,
    pub parallel: Option<u32>,
    pub mod_parallel: Option<u32>,
    pub battleye: Option<bool>,
    pub clear_params: bool,
    pub params: Vec<String>,
    pub json: bool,
}

pub fn cmd_config_set(args: SetArgs) -> Result<()> {
    let mut config = load_config()?;

    if args.clear_arma {
        config.arma_path = None;
    } else if let Some(path) = &args.arma {
        if !config::looks_like_arma(path) {
            anyhow::bail!(
                "{} does not contain arma3_x64.exe or arma3.exe",
                path.display()
            );
        }
        config.arma_path = Some(absolute(path));
    }

    if args.clear_temp {
        config.temp_path = None;
    } else if let Some(path) = &args.temp {
        config.temp_path = Some(absolute(path));
    }

    if args.clear_shared {
        config.shared_path = None;
    } else if let Some(path) = &args.shared {
        config.shared_path = Some(absolute(path));
    }

    if let Some(path) = &args.workspace {
        config.workspace_path = absolute(path);
    }
    if let Some(mode) = &args.link_mode {
        if LinkMode::parse(mode).is_none() {
            anyhow::bail!(
                "unknown link mode '{mode}' (use auto, dir-link, overlay, copy or passthrough)"
            );
        }
        config.link_mode = mode.to_ascii_lowercase();
    }
    if let Some(parallel) = args.parallel {
        if parallel == 0 {
            anyhow::bail!("parallel downloads must be at least 1");
        }
        config.parallel_downloads = parallel;
    }
    if let Some(mods) = args.mod_parallel {
        if mods == 0 {
            anyhow::bail!("parallel mods must be at least 1");
        }
        config.parallel_mods = mods;
    }
    if let Some(battleye) = args.battleye {
        config.battleye = battleye;
    }

    if args.clear_params {
        config.params.clear();
    }
    for raw in &args.params {
        let parsed = LaunchParam::parse(raw);
        if !parsed.is_valid() {
            anyhow::bail!("'{raw}' is not a valid launch parameter (it must start with '-')");
        }
        match config
            .params
            .iter_mut()
            .find(|p| p.key.eq_ignore_ascii_case(&parsed.key))
        {
            Some(existing) => *existing = parsed,
            None => config.params.push(parsed),
        }
    }

    // Create the folders the user pointed at so the UI can show them right away.
    for dir in [
        config.resolved_workspace(),
        config.resolved_temp(),
        config.resolved_shared().unwrap_or_default(),
    ] {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        }
    }

    config.save()?;
    print_snapshot(&config, args.json)
}

fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    }
}

pub fn cmd_repo(action: RepoAction, json_out: bool) -> Result<()> {
    let mut config = load_config()?;

    match action {
        RepoAction::List => {}
        RepoAction::Add {
            url,
            name,
            user,
            pass,
            folder,
            select,
        } => {
            let entry = RepoEntry {
                url: url.clone(),
                name: name.unwrap_or_default(),
                username: user.unwrap_or_default(),
                password: pass.unwrap_or_default(),
                last_checked: None,
                folder,
                path: None,
            };
            let index = config.upsert_repo(entry);
            let resolved = config.repos[index].workspace_under(&config.workspace_path);
            fs::create_dir_all(&resolved)
                .with_context(|| format!("creating {}", resolved.display()))?;
            if select {
                config.select_repo(&url);
            }
            config.save()?;
        }
        RepoAction::Folder {
            url,
            folder,
            default,
        } => {
            if folder.is_some() && default {
                anyhow::bail!("pass either a folder or --default, not both");
            }
            if let Some(folder) = folder {
                if !config.set_repo_folder(&url, Some(folder)) {
                    anyhow::bail!("repo not configured: {url}");
                }
            } else if default && !config.set_repo_folder(&url, None) {
                anyhow::bail!("repo not configured: {url}");
            }
            let resolved = config.workspace_for(&url);
            fs::create_dir_all(&resolved)
                .with_context(|| format!("creating {}", resolved.display()))?;
            config.save()?;
        }
        RepoAction::Remove { url } => {
            if !config.remove_repo(&url) {
                anyhow::bail!("repo not configured: {url}");
            }
            config.save()?;
        }
        RepoAction::Select { url } => {
            if !config.select_repo(&url) {
                anyhow::bail!("repo not configured: {url}");
            }
            config.save()?;
        }
    }

    let snapshot = config_snapshot(&config);
    if json_out {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "repos": snapshot["repos"],
                "activeRepo": snapshot["activeRepo"],
            }))?
        );
        return Ok(());
    }

    let repos = snapshot["repos"].as_array().cloned().unwrap_or_default();
    if repos.is_empty() {
        println!("no repos configured");
        return Ok(());
    }
    for repo in repos {
        println!(
            "{} {}",
            if repo["active"].as_bool().unwrap_or(false) {
                "*"
            } else {
                " "
            },
            repo["name"].as_str().unwrap_or("")
        );
        println!("    {}", repo["url"].as_str().unwrap_or(""));
        let path = repo["path"].as_str().unwrap_or("");
        let folder = repo["folder"].as_str().unwrap_or("");
        let suffix = if repo["autoFolder"].as_bool().unwrap_or(true) {
            " (derived)"
        } else {
            ""
        };
        println!("    {folder}/{suffix} -> {path}");
    }
    Ok(())
}
