//! Repo-side commands: manifest generation, verification, publishing and
//! remote inspection.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::{json, Value};

use nimby_core::formats::{self, generate_from_dir, ModManifest, RepoManifest, RepoMod, SrfFormat};
use nimby_core::graphics;
use nimby_core::hashcache::HashCache;
use nimby_core::http::HttpSource;
use nimby_core::util::format_size;

use crate::settings::load_config;
use crate::split_list;

pub fn cmd_srf(dir: &Path, format: SrfFormat, out: Option<&Path>, json_out: bool) -> Result<()> {
    let name = dir
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "@mod".to_string());
    let manifest = generate_from_dir(&name, dir)?.formatted(format);
    let bytes = manifest.to_bytes(format);
    let target = out
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| dir.join(formats::SRF_FILE_NAME));
    fs::write(&target, &bytes)?;

    if json_out {
        println!(
            "{}",
            json!({
                "name": manifest.name,
                "checksum": manifest.checksum,
                "files": manifest.files.len(),
                "bytes": manifest.total_size(),
                "written": target.display().to_string(),
            })
        );
    } else {
        println!("{} {}", manifest.name, manifest.checksum);
        println!(
            "{} files, {} -> {}",
            manifest.files.len(),
            format_size(manifest.total_size()),
            target.display()
        );
    }
    Ok(())
}

fn load_manifest(dir: &Path, srf: Option<&Path>) -> Result<ModManifest> {
    match srf {
        Some(path) => ModManifest::load(path)
            .with_context(|| format!("cannot read manifest {}", path.display())),
        None => nimby_core::installed_manifest(dir)
            .with_context(|| format!("cannot read manifest for {}", dir.display())),
    }
}

pub fn cmd_verify(dir: &Path, srf: Option<&Path>, strict: bool, json_out: bool) -> Result<()> {
    let manifest = load_manifest(dir, srf)?;
    let cache_path = nimby_core::config::data_dir().join("hashcache.json");
    let cache = std::sync::Mutex::new(HashCache::load(&cache_path));
    let report = nimby_core::verify::verify_dir_cached(&manifest, dir, Some(&cache))?;
    if let Ok(mut cache) = cache.lock() {
        let _ = cache.save();
    }
    let ok = if strict {
        report.is_clean()
    } else {
        report.is_ok()
    };

    if json_out {
        let files: Vec<Value> = report
            .mismatches()
            .map(|f| json!({ "path": f.path, "status": format!("{:?}", f.status) }))
            .collect();
        println!(
            "{}",
            json!({
                "mod": manifest.name,
                "expected": report.expected_checksum,
                "actual": report.actual_checksum,
                "ok": ok,
                "mismatches": files,
                "extraFiles": report.extra_files,
            })
        );
        if !ok {
            std::process::exit(2);
        }
        return Ok(());
    }

    println!("{}", manifest.name);
    println!("  expected {}", report.expected_checksum);
    match &report.actual_checksum {
        Some(actual) => println!("  actual   {actual}"),
        None => println!("  actual   <incomplete>"),
    }
    for check in report.mismatches() {
        println!("  ! {}: {:?}", check.path, check.status);
    }
    for extra in &report.extra_files {
        println!("  + extra {extra}");
    }
    println!(
        "{} ({} files)",
        if ok { "OK" } else { "DIRTY" },
        report.files.len()
    );
    if !ok {
        std::process::exit(2);
    }
    Ok(())
}

fn copy_tree(src: &Path, dst: &Path) -> Result<u64> {
    let mut bytes = 0;
    for entry in walkdir::WalkDir::new(src).follow_links(false) {
        let entry = entry?;
        let rel = entry
            .path()
            .strip_prefix(src)
            .unwrap_or(entry.path())
            .to_path_buf();
        let target = dst.join(rel);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)?;
        } else if entry.file_type().is_file() {
            if entry.file_name() == formats::SRF_FILE_NAME {
                continue;
            }
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(entry.path(), &target)?;
            bytes += entry.metadata()?.len();
        }
    }
    Ok(bytes)
}

#[allow(clippy::too_many_arguments)]
pub fn cmd_publish(
    src: &Path,
    out: &Path,
    config: Option<&Path>,
    name: Option<&str>,
    format: SrfFormat,
    mods: Option<&str>,
    json_out: bool,
) -> Result<()> {
    fs::create_dir_all(out)?;

    let mut repo = match config {
        Some(path) => RepoManifest::load(path)?,
        None => RepoManifest {
            repo_name: name
                .map(|s| s.to_string())
                .unwrap_or_else(|| "Nimby repository".to_string()),
            ..Default::default()
        },
    };
    if let Some(name) = name {
        repo.repo_name = name.to_string();
    }

    let selected = split_list(mods);
    let mut published = Vec::new();

    let mut dirs: Vec<_> = fs::read_dir(src)?
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

    for dir in dirs {
        let mod_name = dir
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        if let Some(list) = &selected {
            if !list.iter().any(|m| m.eq_ignore_ascii_case(&mod_name)) {
                continue;
            }
        }
        let manifest = generate_from_dir(&mod_name, &dir)?.formatted(format);
        let target = out.join(&mod_name);
        fs::create_dir_all(&target)?;
        let bytes = copy_tree(&dir, &target)?;
        manifest.write_to(&target, format)?;
        match repo.mod_by_name(&mod_name) {
            None => repo.required_mods.push(RepoMod {
                mod_name: mod_name.clone(),
                checksum: manifest.checksum.clone(),
                enabled: true,
            }),
            Some(_) => {
                for entry in repo
                    .required_mods
                    .iter_mut()
                    .chain(repo.optional_mods.iter_mut())
                {
                    if entry.mod_name.eq_ignore_ascii_case(&mod_name) {
                        entry.checksum = manifest.checksum.clone();
                    }
                }
            }
        }
        published.push(json!({
            "mod": mod_name,
            "checksum": manifest.checksum,
            "files": manifest.files.len(),
            "bytes": bytes,
        }));
    }

    for (image, key) in [
        ("repo.png", "imageChecksum"),
        ("repo.jpg", "imageChecksum"),
        ("icon.png", "iconImageChecksum"),
        ("icon.jpg", "iconImageChecksum"),
    ] {
        let from = src.join(image);
        if from.is_file() {
            let to = out.join(image);
            fs::copy(&from, &to)?;
            repo.extra.insert(
                key.to_string(),
                Value::String(nimby_core::checksum::sha1_hex(&fs::read(&to)?)),
            );
        }
    }

    fs::write(out.join("repo.json"), repo.to_json())?;

    if json_out {
        println!(
            "{}",
            json!({ "repoName": repo.repo_name, "published": published })
        );
    } else {
        println!("published {} mods to {}", published.len(), out.display());
    }
    Ok(())
}

/// Build an HTTP source for a repo, using stored credentials when available.
pub fn repo_http(url: &str) -> Result<HttpSource> {
    let config = load_config().unwrap_or_default();
    let normalized = nimby_core::config::normalize_url(url);
    let auth = config
        .repos
        .iter()
        .find(|r| r.url == normalized && !r.username.is_empty())
        .map(|r| (r.username.clone(), r.password.clone()));
    Ok(HttpSource::with_auth(url, auth)?)
}

pub fn cmd_inspect(
    repo: &str,
    mods: Option<&str>,
    with_images: bool,
    json_out: bool,
) -> Result<()> {
    let http = repo_http(repo)?;
    let text = http.get("repo.json")?;
    let manifest = RepoManifest::parse(&text)?;
    let selected = split_list(mods);

    let images = if with_images {
        // The artwork cache follows the configuration: next to the executable in
        // a portable install, otherwise under the user profile.
        Some(graphics::fetch_images(
            &http,
            &nimby_core::config::data_dir(),
        )?)
    } else {
        None
    };

    let mut rows = Vec::new();
    let required_names: Vec<&str> = manifest
        .required_mods
        .iter()
        .map(|m| m.mod_name.as_str())
        .collect();
    for entry in manifest.all_mods() {
        if let Some(list) = &selected {
            if !list.iter().any(|m| m.eq_ignore_ascii_case(&entry.mod_name)) {
                continue;
            }
        }
        let srf = http
            .get(&format!("{}/mod.srf", entry.mod_name))
            .ok()
            .and_then(|bytes| ModManifest::parse(&bytes).ok());
        let is_required = required_names
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&entry.mod_name));
        rows.push(json!({
            "mod": entry.mod_name,
            "checksum": entry.checksum,
            "enabled": entry.enabled,
            "required": is_required,
            "optional": !is_required,
            "files": srf.as_ref().map(|m| m.files.len()),
            "bytes": srf.as_ref().map(|m| m.total_size()),
            "srfFormat": srf.as_ref().map(|m| match m.format {
                SrfFormat::V3 => "v3",
                SrfFormat::Legacy => "legacy",
            }),
        }));
    }

    let servers: Vec<Value> = manifest
        .servers
        .iter()
        .map(|s| {
            json!({
                "name": s.name,
                "address": s.address,
                "port": s.port,
                "battleEye": s.battle_eye,
                "hasPassword": !s.password.is_empty(),
            })
        })
        .collect();

    if json_out {
        let images_json = match &images {
            Some(images) => json!({
                "repo": images.repo_image,
                "icon": images.icon_image,
                "repoSha1": images.repo_sha1,
                "iconSha1": images.icon_sha1,
            }),
            None => Value::Null,
        };
        println!(
            "{}",
            json!({
                "repoName": manifest.repo_name,
                "version": manifest.version,
                "clientParameters": manifest.client_parameters,
                "servers": servers,
                "images": images_json,
                "mods": rows,
            })
        );
        return Ok(());
    }

    println!(
        "{} ({})",
        manifest.repo_name,
        manifest.version.clone().unwrap_or_default()
    );
    println!("{} mods", rows.len());
    for row in &rows {
        println!(
            "  {:<32} {:<34} {:>6} files  {:>10}",
            row["mod"].as_str().unwrap_or_default(),
            row["checksum"].as_str().unwrap_or_default(),
            row["files"].as_u64().unwrap_or(0),
            format_size(row["bytes"].as_u64().unwrap_or(0)),
        );
    }
    if let Some(images) = &images {
        if let Some(path) = &images.repo_image {
            println!("banner {path}");
        }
        if let Some(path) = &images.icon_image {
            println!("icon   {path}");
        }
    }
    Ok(())
}
