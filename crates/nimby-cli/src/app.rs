//! Workspace commands: install, status, stats, doctor and launch.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{json, Value};

use nimby_core::config::{self, AppConfig};
use nimby_core::formats::ModManifest;
use nimby_core::hashcache::HashCache;
use nimby_core::install::{install_mod, InstallOptions, InstallReport, ProgressEvent};
use nimby_core::launch::{self, LaunchOptions};
use nimby_core::linker::{self, LinkMode};
use nimby_core::params;
use nimby_core::stats;
use nimby_core::steam;
use nimby_core::util::format_size;

use crate::control::{Action, Control, OnError};
use crate::settings::load_config;
use crate::{split_list, ProgressArg};

/// Human readable transfer rate, e.g. `12.4 MB/s`.
fn format_speed(bytes_per_second: u64) -> String {
    format!("{}/s", format_size(bytes_per_second))
}

/// Exit codes so the GUI and scripts can tell the outcomes apart.
pub const EXIT_CANCELLED: i32 = 3;
pub const EXIT_INCOMPLETE: i32 = 4;

fn resolve_repo_url(config: &AppConfig, explicit: Option<&str>) -> Result<String> {
    if let Some(url) = explicit {
        return Ok(config::normalize_url(url));
    }
    if let Some(active) = config.active_repo() {
        return Ok(active.url.clone());
    }
    anyhow::bail!(
        "no repository given and none selected; use `nimby repo add <url> --select` or pass a URL"
    )
}

/// Workshop roots for one mod, matched through its `meta.cpp` publishedid.
fn workshop_roots(
    http: &nimby_core::http::HttpSource,
    mod_name: &str,
    items: &[steam::WorkshopItem],
) -> Vec<PathBuf> {
    let Ok(meta) = http.get(&format!("{mod_name}/meta.cpp")) else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&meta);
    let Some(published_id) = steam::parse_meta_cpp(&text).published_id else {
        return Vec::new();
    };
    items
        .iter()
        .filter(|item| item.published_id == published_id)
        .map(|item| item.root.clone())
        .collect()
}

#[allow(clippy::too_many_arguments)]
pub fn cmd_install(
    repo: Option<&str>,
    path: Option<&Path>,
    mods: Option<&str>,
    optional: Option<&str>,
    no_required: bool,
    mode: Option<&str>,
    no_workshop: bool,
    no_store: bool,
    force: bool,
    progress: Option<ProgressArg>,
    parallel: Option<u32>,
    mod_parallel: Option<u32>,
    on_error: OnError,
    control: Control,
    max_retries: u32,
    json_out: bool,
) -> Result<()> {
    let config = load_config()?;
    let repo_url = resolve_repo_url(&config, repo)?;
    // Each repository has its own folder; `--path` still wins for one-off runs.
    let workspace = path
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| config.workspace_for(&repo_url));

    let link_mode = match mode {
        Some(value) => {
            LinkMode::parse(value).ok_or_else(|| anyhow::anyhow!("unknown link mode '{value}'"))?
        }
        None => config.link_mode(),
    };
    let store = if no_store {
        None
    } else {
        config.resolved_shared()
    };
    if let Some(store) = &store {
        fs::create_dir_all(store)?;
    }
    fs::create_dir_all(&workspace)?;

    let max_parallel = parallel.unwrap_or(config.parallel_downloads).clamp(1, 32) as usize;
    let mod_parallel = mod_parallel
        .map(|n| n as usize)
        .unwrap_or(config.parallel_mods as usize)
        .clamp(1, 16);

    // Persistent hash cache: unchanged Workshop/store files are not rehashed.
    let cache_path = config::data_dir().join("hashcache.json");
    let cache = std::sync::Mutex::new(HashCache::load(&cache_path));

    let http = crate::publish::repo_http(&repo_url)?;
    let manifest = nimby_core::RepoManifest::parse(&http.get_retrying("repo.json", max_retries)?)?;

    let selected = split_list(mods);
    let optional_selected = split_list(optional).unwrap_or_default();
    let mut wanted = Vec::new();
    for entry in manifest.all_mods() {
        let is_required = manifest
            .required_mods
            .iter()
            .any(|m| m.mod_name.eq_ignore_ascii_case(&entry.mod_name));
        let pick = match &selected {
            Some(list) => list.iter().any(|m| m.eq_ignore_ascii_case(&entry.mod_name)),
            None => !no_required && is_required && entry.enabled,
        };
        let optional_pick = optional_selected
            .iter()
            .any(|m| m.eq_ignore_ascii_case(&entry.mod_name));
        if pick || optional_pick {
            wanted.push(entry.clone());
        }
    }
    if wanted.is_empty() {
        anyhow::bail!("nothing selected to install");
    }

    // Phase 1: resolve every mod.srf up front, so progress can be measured
    // against the real total byte count.
    let emitter = Emitter::new(progress);
    let mut plans: Vec<(nimby_core::RepoMod, ModManifest)> = Vec::with_capacity(wanted.len());
    for (index, entry) in wanted.iter().enumerate() {
        if control.is_cancelled() {
            emitter.cancelled(&[], &[]);
            std::process::exit(EXIT_CANCELLED);
        }
        emitter.resolving(&entry.mod_name, index + 1, wanted.len());
        let srf = http
            .get_retrying(&format!("{}/mod.srf", entry.mod_name), max_retries)
            .with_context(|| format!("fetching mod.srf for {}", entry.mod_name))?;
        let mod_manifest = ModManifest::parse(&srf)?;
        if !entry.checksum.is_empty()
            && !mod_manifest.checksum.eq_ignore_ascii_case(&entry.checksum)
        {
            emitter.note(&format!(
                "{} checksum differs from repo.json ({} vs {})",
                entry.mod_name, mod_manifest.checksum, entry.checksum
            ));
        }
        plans.push((entry.clone(), mod_manifest));
    }

    let total_bytes: u64 = plans.iter().map(|(_, m)| m.total_size()).sum();
    let total_files: usize = plans.iter().map(|(_, m)| m.files.len()).sum();
    emitter.plan(&repo_url, plans.len(), total_files, total_bytes);

    let workshop_items = if no_workshop {
        Vec::new()
    } else {
        steam::scan_workshop(steam::ARMA3_APP_ID)
    };

    let mut total_reused = 0u64;
    let mut total_store = 0u64;
    let mut total_downloaded = 0u64;

    // The tracking index lives in the install folder and records, per file,
    // where the bytes came from. It is written after every mod so an interrupted
    // run still leaves accurate statistics behind.
    let mut index = nimby_core::tracking::RepoIndex::load(&workspace);
    index.repo = repo_url.clone();
    let tracking = std::sync::Mutex::new(index);

    // Mods are installed **in parallel** as well: a work queue is drained by
    // `parallel_mods` workers, and each worker keeps the per-file parallelism it
    // had before (sharing the download budget between the active mods).
    let mod_count = plans.len();
    let workers = mod_parallel.min(mod_count).max(1);
    // Every active mod keeps the full per-file budget: chunk transfers are what
    // saturates a link, and giving each mod fewer workers was measurably slower.
    // Total connections are still bounded by `workers * max_parallel`.
    let per_mod_parallel = max_parallel;
    let queue = std::sync::Mutex::new((0..mod_count).collect::<std::collections::VecDeque<_>>());
    let results: std::sync::Mutex<Vec<Option<InstallReport>>> =
        std::sync::Mutex::new((0..mod_count).map(|_| None).collect());
    let skipped: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());
    let failure: std::sync::Mutex<Option<(String, String)>> = std::sync::Mutex::new(None);
    let stopped_at: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                if control.is_cancelled() || failure.lock().map(|f| f.is_some()).unwrap_or(false) {
                    break;
                }
                let Some(index) = queue.lock().ok().and_then(|mut queue| queue.pop_front()) else {
                    break;
                };
                let (entry, mod_manifest) = &plans[index];
                let mod_name = &entry.mod_name;

                let roots = if no_workshop {
                    Vec::new()
                } else {
                    workshop_roots(&http, mod_name, &workshop_items)
                };

                emitter.mod_start(
                    mod_name,
                    index + 1,
                    mod_count,
                    mod_manifest.total_size(),
                    mod_manifest.files.len(),
                );

                let target = workspace.join(mod_name);
                let mut tries = 0u32;
                // Attempts the user asked for explicitly, as opposed to automatic ones.
                let mut manual = 0u32;
                let report = loop {
                    let callback = |event: nimby_core::ProgressEvent<'_>| emitter.file(event);
                    let opts = InstallOptions {
                        mode: link_mode,
                        workshop_roots: &roots,
                        store_root: store.as_deref(),
                        http: Some(&http),
                        local_repo: None,
                        repo: Some(&repo_url),
                        force,
                        on_progress: Some(&callback),
                        max_parallel: per_mod_parallel,
                        cache: Some(&cache),
                        cancel: Some(control.cancel_flag()),
                        retries: max_retries.max(1),
                    };
                    match install_mod(mod_manifest, &target, &opts) {
                        Ok(report) => break Some(report),
                        Err(err) if err.is_cancelled() => {
                            if let Ok(mut stopped) = stopped_at.lock() {
                                if stopped.is_none() {
                                    *stopped = Some(mod_name.clone());
                                }
                            }
                            break None;
                        }
                        Err(err) => {
                            // Automatic retry for transient problems; the
                            // installer keeps its partial work, so retrying
                            // resumes.
                            if err.is_transient() && tries < max_retries {
                                tries += 1;
                                let delay = crate::control::backoff(tries);
                                emitter.retry(
                                    mod_name,
                                    tries,
                                    max_retries,
                                    &err.to_string(),
                                    delay.as_millis() as u64,
                                );
                                std::thread::sleep(delay);
                                continue;
                            }
                            let action = match on_error {
                                OnError::Abort => Action::Abort,
                                OnError::Skip => Action::Skip,
                                OnError::Retry => {
                                    // Never spin forever on a permanent error:
                                    // after a few manual retries it will not get
                                    // better.
                                    if manual < max_retries.max(1) * 2 {
                                        Action::Retry
                                    } else {
                                        emitter.note(&format!(
                                            "giving up on {mod_name} after {manual} manual retries"
                                        ));
                                        Action::Abort
                                    }
                                }
                                OnError::Ask if control.can_ask() => {
                                    emitter.needs_action(
                                        mod_name,
                                        &err.to_string(),
                                        err.kind_label(),
                                    );
                                    let action = control.wait_answer();
                                    emitter.note(&format!("user chose to {}", action.as_str()));
                                    action
                                }
                                OnError::Ask => Action::Abort,
                            };
                            match action {
                                Action::Retry => {
                                    manual += 1;
                                    tries = 0;
                                    continue;
                                }
                                Action::Skip => {
                                    emitter.skipped(mod_name, &err.to_string());
                                    if let Ok(mut skipped) = skipped.lock() {
                                        skipped.push((mod_name.clone(), err.to_string()));
                                    }
                                    break None;
                                }
                                Action::Abort => {
                                    if !control.is_cancelled() {
                                        if let Ok(mut failure) = failure.lock() {
                                            if failure.is_none() {
                                                *failure =
                                                    Some((mod_name.clone(), err.to_string()));
                                            }
                                        }
                                    } else if let Ok(mut stopped) = stopped_at.lock() {
                                        if stopped.is_none() {
                                            *stopped = Some(mod_name.clone());
                                        }
                                    }
                                    // Stop the other workers instead of letting
                                    // them start new mods.
                                    control.request_cancel();
                                    break None;
                                }
                            }
                        }
                    }
                };

                let Some(report) = report else {
                    continue;
                };

                if let Ok(mut tracking) = tracking.lock() {
                    tracking.record(
                        &workspace,
                        Some(&repo_url),
                        mod_manifest,
                        &report.mode,
                        &report.files,
                    );
                    let _ = tracking.save(&workspace);
                }
                emitter.mod_done(&report);
                if let Ok(mut results) = results.lock() {
                    results[index] = Some(report);
                }
            });
        }
    });

    // Collect in plan order so the report is stable however the workers raced.
    let mut reports: Vec<InstallReport> = Vec::with_capacity(mod_count);
    let mut pending: Vec<String> = Vec::new();
    if let Ok(mut results) = results.lock() {
        for (index, slot) in results.iter_mut().enumerate() {
            match slot.take() {
                Some(report) => reports.push(report),
                None => pending.push(plans[index].0.mod_name.clone()),
            }
        }
    }
    let installed: Vec<String> = reports.iter().map(|r| r.mod_name.clone()).collect();
    let skipped = skipped.into_inner().unwrap_or_default();
    let failure = failure.into_inner().unwrap_or(None);
    let stopped = stopped_at.into_inner().unwrap_or(None);

    for report in &reports {
        total_reused += report.reused_bytes;
        total_store += report.store_bytes;
        total_downloaded += report.downloaded_bytes;
    }

    let saved = total_reused + total_store;

    // Persist the hash cache so the next run skips unchanged files.
    if let Ok(mut cache) = cache.lock() {
        let _ = cache.save();
    }

    if let Some(mod_name) = stopped {
        emitter.cancelled(&installed, &pending);
        if !emitter.streaming() {
            eprintln!(
                "cancelled at {mod_name}; {}/{} mods done, re-run to resume (finished files are kept)",
                installed.len(),
                plans.len()
            );
        }
        let _ = std::io::Write::flush(&mut std::io::stdout());
        std::process::exit(EXIT_CANCELLED);
    }

    if let Some((mod_name, message)) = failure {
        emitter.failed(&mod_name, &message);
        anyhow::bail!("{mod_name} could not be installed: {message}");
    }

    emitter.done(plans.len(), saved, total_downloaded);

    if !skipped.is_empty() {
        if !emitter.streaming() {
            for (mod_name, message) in &skipped {
                eprintln!("skipped {mod_name}: {message}");
            }
        }
        let _ = std::io::Write::flush(&mut std::io::stdout());
        std::process::exit(EXIT_INCOMPLETE);
    }

    if emitter.streaming() {
        return Ok(());
    }

    if json_out {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "repo": repo_url,
                "workspace": workspace.display().to_string(),
                "store": store.as_ref().map(|p| p.display().to_string()),
                "installed": reports,
                "reusedBytes": total_reused,
                "storeBytes": total_store,
                "downloadedBytes": total_downloaded,
                "savedBytes": saved,
            }))?
        );
    } else {
        for report in &reports {
            println!(
                "  {:<32} {:<10} linked {:>4}  store {:>4}  net {:>4}  saved {:>10}",
                report.mod_name,
                report.mode,
                report.linked_files,
                report.store_files,
                report.downloaded_files,
                format_size(report.saved_bytes()),
            );
        }
        println!(
            "saved {} (workshop {} + store {}), downloaded {}",
            format_size(saved),
            format_size(total_reused),
            format_size(total_store),
            format_size(total_downloaded)
        );
    }
    Ok(())
}

/// Progress output: JSON lines for the GUI, compact text for terminals.
///
/// Callbacks fire from several download threads, so counters are atomic and the
/// per-file partial amounts live behind a mutex.
struct Emitter {
    mode: Option<ProgressArg>,
    total_bytes: std::sync::atomic::AtomicU64,
    total_files: std::sync::atomic::AtomicUsize,
    completed_bytes: std::sync::atomic::AtomicU64,
    completed_files: std::sync::atomic::AtomicU64,
    /// `mod\u{1}path` -> bytes of that file that already arrived.
    partials: std::sync::Mutex<std::collections::HashMap<String, u64>>,
    active: std::sync::atomic::AtomicU64,
    /// Bytes that actually came over the network, for the speed meter.
    net_bytes: std::sync::atomic::AtomicU64,
    /// Recent `(time, net_bytes)` samples used to derive the current speed.
    samples: std::sync::Mutex<std::collections::VecDeque<(std::time::Instant, u64)>>,
    /// Smoothed speed in bytes per second.
    speed: std::sync::atomic::AtomicU64,
    started: std::time::Instant,
}

fn partial_key(mod_name: &str, path: &str) -> String {
    format!("{mod_name}\u{1}{path}")
}

use std::sync::atomic::Ordering;

impl Emitter {
    fn new(mode: Option<ProgressArg>) -> Self {
        Self {
            mode,
            total_bytes: std::sync::atomic::AtomicU64::new(0),
            total_files: std::sync::atomic::AtomicUsize::new(0),
            completed_bytes: std::sync::atomic::AtomicU64::new(0),
            completed_files: std::sync::atomic::AtomicU64::new(0),
            partials: std::sync::Mutex::new(std::collections::HashMap::new()),
            active: std::sync::atomic::AtomicU64::new(0),
            net_bytes: std::sync::atomic::AtomicU64::new(0),
            samples: std::sync::Mutex::new(std::collections::VecDeque::new()),
            speed: std::sync::atomic::AtomicU64::new(0),
            started: std::time::Instant::now(),
        }
    }

    /// Fold freshly received bytes into the speed meter.
    fn record_network(&self, delta: u64) {
        if delta == 0 {
            return;
        }
        self.net_bytes.fetch_add(delta, Ordering::Relaxed);
        let total = self.net_bytes.load(Ordering::Relaxed);
        let now = std::time::Instant::now();
        let Ok(mut samples) = self.samples.lock() else {
            return;
        };
        samples.push_back((now, total));
        // Keep a ~4 s window, but always keep at least two samples.
        while samples.len() > 2 {
            match samples.front() {
                Some((first, _)) if now.duration_since(*first).as_secs_f64() > 4.0 => {
                    samples.pop_front();
                }
                _ => break,
            }
        }
        let (Some((t0, _)), Some((t1, b1))) = (samples.front(), samples.back()) else {
            return;
        };
        let seconds = t1.duration_since(*t0).as_secs_f64();
        if seconds < 0.2 {
            return;
        }
        let instant = (*b1 as f64 / seconds) as u64;
        let previous = self.speed.load(Ordering::Relaxed);
        // Exponential moving average so the number does not flicker.
        let smoothed = if previous == 0 {
            instant
        } else {
            (previous as f64 * 0.6 + instant as f64 * 0.4) as u64
        };
        self.speed.store(smoothed, Ordering::Relaxed);
    }

    /// Current download speed in bytes per second (0 when idle).
    fn current_speed(&self) -> u64 {
        // A stalled transfer must not keep showing its last rate.
        let idle_seconds = self
            .samples
            .lock()
            .ok()
            .and_then(|samples| samples.back().map(|(at, _)| at.elapsed().as_secs_f64()))
            .unwrap_or(f64::MAX);
        if idle_seconds > 2.0 {
            return 0;
        }
        self.speed.load(Ordering::Relaxed)
    }

    /// Seconds left at the current speed, 0 when unknown.
    fn eta_seconds(&self) -> u64 {
        let speed = self.current_speed();
        if speed == 0 {
            return 0;
        }
        self.total().saturating_sub(self.done_bytes()) / speed
    }

    /// Average speed over the whole run.
    fn average_speed(&self) -> u64 {
        let network = self.net_bytes.load(Ordering::Relaxed);
        if network == 0 {
            return 0;
        }
        // Very short runs still deserve a number; clamp the divisor instead of
        // reporting a huge rate.
        let seconds = self.started.elapsed().as_secs_f64().max(0.2);
        (network as f64 / seconds) as u64
    }

    fn streaming(&self) -> bool {
        self.mode.is_some()
    }

    fn is_json(&self) -> bool {
        matches!(self.mode, Some(ProgressArg::Json))
    }

    fn line(&self, value: Value) {
        if self.is_json() {
            // One println per event keeps lines from interleaving across threads.
            println!("{value}");
            let _ = std::io::Write::flush(&mut std::io::stdout());
        }
    }

    fn total(&self) -> u64 {
        self.total_bytes.load(Ordering::Relaxed)
    }

    /// Bytes across finished files plus the partial progress of active ones.
    fn done_bytes(&self) -> u64 {
        let completed = self.completed_bytes.load(Ordering::Relaxed);
        let partial: u64 = match self.partials.lock() {
            Ok(map) => map.values().sum(),
            Err(poisoned) => poisoned.into_inner().values().sum(),
        };
        completed + partial
    }

    fn percent(&self) -> u64 {
        self.done_bytes()
            .saturating_mul(100)
            .checked_div(self.total())
            .unwrap_or(0)
            .min(100)
    }

    fn resolving(&self, mod_name: &str, index: usize, count: usize) {
        match self.mode {
            Some(ProgressArg::Json) => self.line(json!({
                "event": "resolving",
                "mod": mod_name,
                "index": index,
                "count": count,
            })),
            Some(ProgressArg::Text) => println!("[{index}/{count}] resolving {mod_name}…"),
            None => {}
        }
    }

    fn plan(&self, repo: &str, mods: usize, files: usize, bytes: u64) {
        self.total_bytes.store(bytes, Ordering::Relaxed);
        self.total_files.store(files, Ordering::Relaxed);
        match self.mode {
            Some(ProgressArg::Json) => self.line(json!({
                "event": "plan",
                "repo": repo,
                "mods": mods,
                "files": files,
                "totalBytes": bytes,
            })),
            Some(ProgressArg::Text) => println!(
                "planning {mods} mods · {files} files · {}",
                format_size(bytes)
            ),
            None => {}
        }
    }

    fn mod_start(&self, mod_name: &str, index: usize, count: usize, bytes: u64, files: usize) {
        match self.mode {
            Some(ProgressArg::Json) => self.line(json!({
                "event": "modStart",
                "mod": mod_name,
                "index": index,
                "count": count,
                "bytes": bytes,
                "files": files,
                "percent": self.percent(),
            })),
            Some(ProgressArg::Text) => println!(
                "[{:>3}%] {index}/{count} {mod_name} ({} files, {})",
                self.percent(),
                files,
                format_size(bytes)
            ),
            None => {}
        }
    }

    fn file_start(&self, mod_name: &str, path: &str, bytes: u64) {
        if let Ok(mut map) = self.partials.lock() {
            map.insert(partial_key(mod_name, path), 0);
        }
        self.active.fetch_add(1, Ordering::Relaxed);
        if self.is_json() {
            self.line(json!({
                "event": "fileStart",
                "mod": mod_name,
                "path": path,
                "bytes": bytes,
                "active": self.active.load(Ordering::Relaxed),
                "doneBytes": self.done_bytes(),
                "totalBytes": self.total(),
                "percent": self.percent(),
            }));
        }
    }

    fn file_bytes(&self, mod_name: &str, path: &str, done: u64) {
        let mut delta = 0u64;
        if let Ok(mut map) = self.partials.lock() {
            let previous = map.insert(partial_key(mod_name, path), done).unwrap_or(0);
            delta = done.saturating_sub(previous);
        }
        // Only the growth of a partial transfer is network traffic.
        self.record_network(delta);
        if self.is_json() {
            self.line(json!({
                "event": "fileProgress",
                "mod": mod_name,
                "path": path,
                "fileBytes": done,
                "active": self.active.load(Ordering::Relaxed),
                "doneBytes": self.done_bytes(),
                "totalBytes": self.total(),
                "percent": self.percent(),
                "speed": self.current_speed(),
                "etaSeconds": self.eta_seconds(),
            }));
        }
    }

    fn file(&self, event: ProgressEvent<'_>) {
        match event {
            ProgressEvent::FileStart {
                mod_name,
                path,
                bytes,
            } => {
                self.file_start(mod_name, path, bytes);
                return;
            }
            ProgressEvent::FileBytes {
                mod_name,
                path,
                done,
                ..
            } => {
                self.file_bytes(mod_name, path, done);
                return;
            }
            ProgressEvent::Note { message } => {
                self.note(message);
                return;
            }
            ProgressEvent::Retry {
                mod_name,
                attempt,
                attempts,
                message,
                ..
            } => {
                self.retry_event(mod_name, attempt, attempts, message);
                return;
            }
            ProgressEvent::Resumed {
                mod_name,
                files,
                bytes,
            } => {
                self.resumed(mod_name, files, bytes);
                return;
            }
            _ => {}
        }

        let (mod_name, path, action, bytes) = match event {
            ProgressEvent::File {
                mod_name,
                path,
                action,
                bytes,
            } => (mod_name, path.to_string(), action, bytes),
            ProgressEvent::ModLinked {
                mod_name,
                action,
                bytes,
            } => (mod_name, "(whole mod)".to_string(), action, bytes),
            _ => unreachable!("handled above"),
        };

        if let Ok(mut map) = self.partials.lock() {
            if let Some(partial) = map.remove(&partial_key(mod_name, &path)) {
                // The partial transfer was already counted as it arrived; only
                // the remainder is new network traffic.
                if action == nimby_core::install::FileAction::Download {
                    self.record_network(bytes.saturating_sub(partial));
                }
            } else if action == nimby_core::install::FileAction::Download {
                self.record_network(bytes);
            }
        }
        self.completed_bytes.fetch_add(bytes, Ordering::Relaxed);
        self.completed_files.fetch_add(1, Ordering::Relaxed);
        let active = self.active.load(Ordering::Relaxed);
        if active > 0 {
            self.active.fetch_sub(1, Ordering::Relaxed);
        }

        if self.is_json() {
            self.line(json!({
                "event": "file",
                "mod": mod_name,
                "path": path,
                "action": action,
                "bytes": bytes,
                "active": self.active.load(Ordering::Relaxed),
                "doneBytes": self.done_bytes(),
                "totalBytes": self.total(),
                "doneFiles": self.completed_files.load(Ordering::Relaxed),
                "totalFiles": self.total_files.load(Ordering::Relaxed),
                "percent": self.percent(),
                "speed": self.current_speed(),
                "etaSeconds": self.eta_seconds(),
            }));
        }
    }

    fn mod_done(&self, report: &InstallReport) {
        match self.mode {
            Some(ProgressArg::Json) => self.line(json!({
                "event": "modDone",
                "mod": report.mod_name,
                "mode": report.mode,
                "savedBytes": report.saved_bytes(),
                "downloadedBytes": report.downloaded_bytes,
                "percent": self.percent(),
                "averageSpeed": self.average_speed(),
            })),
            Some(ProgressArg::Text) => println!(
                "         {} done · {} (saved {})",
                report.mod_name,
                report.mode,
                format_size(report.saved_bytes())
            ),
            None => {}
        }
    }

    fn note(&self, message: &str) {
        match self.mode {
            Some(ProgressArg::Json) => self.line(json!({ "event": "note", "message": message })),
            Some(ProgressArg::Text) => println!("note: {message}"),
            None => eprintln!("warning: {message}"),
        }
    }

    fn done(&self, mods: usize, saved: u64, downloaded: u64) {
        match self.mode {
            Some(ProgressArg::Json) => self.line(json!({
                "event": "done",
                "mods": mods,
                "doneBytes": self.done_bytes(),
                "totalBytes": self.total(),
                "savedBytes": saved,
                "downloadedBytes": downloaded,
                "percent": 100,
                "averageSpeed": self.average_speed(),
                "elapsedMs": self.started.elapsed().as_millis() as u64,
            })),
            Some(ProgressArg::Text) => println!(
                "done: {mods} mods · saved {} · downloaded {} · avg {}",
                format_size(saved),
                format_size(downloaded),
                format_speed(self.average_speed())
            ),
            None => {}
        }
    }

    /// A transfer failed and the installer is retrying it.
    fn retry(&self, mod_name: &str, attempt: u32, attempts: u32, message: &str, delay_ms: u64) {
        match self.mode {
            Some(ProgressArg::Json) => self.line(json!({
                "event": "retry",
                "mod": mod_name,
                "attempt": attempt,
                "attempts": attempts,
                "delayMs": delay_ms,
                "message": message,
            })),
            Some(ProgressArg::Text) => {
                println!("retrying {mod_name} ({attempt}/{attempts}) in {delay_ms} ms: {message}")
            }
            None => eprintln!("retrying {mod_name} ({attempt}/{attempts}): {message}"),
        }
    }

    /// A retry that came from inside the installer (a single file transfer).
    fn retry_event(&self, mod_name: &str, attempt: u32, attempts: u32, message: &str) {
        match self.mode {
            Some(ProgressArg::Json) => self.line(json!({
                "event": "retry",
                "mod": mod_name,
                "attempt": attempt,
                "attempts": attempts,
                "message": message,
            })),
            Some(ProgressArg::Text) => {
                println!("retrying {mod_name} ({attempt}/{attempts}): {message}")
            }
            None => eprintln!("retrying {mod_name} ({attempt}/{attempts}): {message}"),
        }
    }

    fn resumed(&self, mod_name: &str, files: usize, bytes: u64) {
        match self.mode {
            Some(ProgressArg::Json) => self.line(json!({
                "event": "resumed",
                "mod": mod_name,
                "files": files,
                "bytes": bytes,
                "percent": self.percent(),
            })),
            Some(ProgressArg::Text) => println!(
                "{mod_name}: kept {files} file(s) from the previous run ({})",
                format_size(bytes)
            ),
            None => {}
        }
    }

    /// Ask the user what to do. The install blocks until an answer arrives.
    fn needs_action(&self, mod_name: &str, message: &str, kind: &str) {
        let _ = std::io::Write::flush(&mut std::io::stdout());
        match self.mode {
            Some(ProgressArg::Json) => self.line(json!({
                "event": "needsAction",
                "mod": mod_name,
                "kind": kind,
                "message": message,
                "options": ["retry", "skip", "abort"],
                "percent": self.percent(),
            })),
            _ => eprintln!("{mod_name} needs a decision: {message}"),
        }
    }

    fn skipped(&self, mod_name: &str, message: &str) {
        match self.mode {
            Some(ProgressArg::Json) => self.line(json!({
                "event": "skipped",
                "mod": mod_name,
                "message": message,
                "percent": self.percent(),
            })),
            Some(ProgressArg::Text) => println!("skipped {mod_name}: {message}"),
            None => eprintln!("skipped {mod_name}: {message}"),
        }
    }

    /// The user cancelled. Everything already installed stays on disk.
    fn cancelled(&self, installed: &[String], pending: &[String]) {
        match self.mode {
            Some(ProgressArg::Json) => self.line(json!({
                "event": "cancelled",
                "installed": installed,
                "pending": pending,
                "doneBytes": self.done_bytes(),
                "totalBytes": self.total(),
                "percent": self.percent(),
                "resumable": true,
            })),
            Some(ProgressArg::Text) => println!(
                "cancelled: {} mod(s) installed, {} still pending (re-run to resume)",
                installed.len(),
                pending.len()
            ),
            None => eprintln!(
                "cancelled: {} mod(s) installed, {} still pending",
                installed.len(),
                pending.len()
            ),
        }
    }

    fn failed(&self, mod_name: &str, message: &str) {
        match self.mode {
            Some(ProgressArg::Json) => self.line(json!({
                "event": "failed",
                "mod": mod_name,
                "message": message,
                "percent": self.percent(),
            })),
            _ => eprintln!("{mod_name} failed: {message}"),
        }
    }
}

/// Last-seen state of an installed mod, used by `update` and `repair`.
pub struct InstalledMod {
    pub name: String,
    pub checksum: String,
    pub repo: String,
    pub target: PathBuf,
}

pub fn installed_mods(workspace: &Path) -> Result<Vec<InstalledMod>> {
    let mut out = Vec::new();
    for dir in nimby_core::workspace::mod_dirs(workspace).unwrap_or_default() {
        let Some(lock) = nimby_core::read_lock(&dir) else {
            continue;
        };
        out.push(InstalledMod {
            name: lock.mod_name.clone(),
            checksum: lock.checksum.clone(),
            repo: lock.repo.clone().unwrap_or_default(),
            target: dir,
        });
    }
    Ok(out)
}

pub fn cmd_update(check: bool, deep: bool, json_out: bool) -> Result<()> {
    let config = load_config()?;
    let cache_path = config::data_dir().join("hashcache.json");
    let cache = std::sync::Mutex::new(HashCache::load(&cache_path));
    let mut installed = Vec::new();
    let mut folders = Vec::new();
    for (_, workspace) in config.workspaces() {
        folders.push(workspace.clone());
        installed.extend(installed_mods(&workspace)?);
    }
    let installed_count = installed.len();
    if installed.is_empty() {
        anyhow::bail!(
            "no installed mods in {}",
            folders
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    // repo -> installed entries
    let mut groups: std::collections::BTreeMap<String, Vec<InstalledMod>> = Default::default();
    for entry in installed {
        if entry.repo.is_empty() {
            continue;
        }
        groups.entry(entry.repo.clone()).or_default().push(entry);
    }

    let mut changed: Vec<Value> = Vec::new();
    // (repo, folder) -> mods to reinstall, because every repository can live in
    // its own folder and a repo may even be installed in more than one.
    let mut wanted: std::collections::BTreeMap<(String, PathBuf), Vec<String>> = Default::default();
    for (repo, mods) in &groups {
        let http = crate::publish::repo_http(repo)?;
        let manifest = nimby_core::RepoManifest::parse(&http.get_retrying("repo.json", 3)?)?;
        for entry in mods {
            let folder = entry
                .target
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| config.workspace_for(repo));
            // The tracking index already knows which checksum is installed, so
            // repo.json is enough for the fast path: no mod.srf round trip.
            let index = nimby_core::tracking::RepoIndex::load(&folder);
            let tracked = index.mods.get(&entry.name);
            let remote = tracked
                .map(|_| {
                    manifest
                        .mod_by_name(&entry.name)
                        .map(|m| m.checksum.clone())
                        .unwrap_or_default()
                })
                .filter(|checksum| !checksum.is_empty())
                .or_else(|| {
                    http.get(&format!("{}/mod.srf", entry.name))
                        .ok()
                        .and_then(|bytes| ModManifest::parse(&bytes).ok())
                        .map(|m| m.checksum)
                })
                .or_else(|| {
                    manifest
                        .mod_by_name(&entry.name)
                        .map(|m| m.checksum.clone())
                })
                .unwrap_or_default();
            if remote.is_empty() {
                continue;
            }

            let mut state = if tracked.is_none() {
                nimby_core::tracking::UpdateState::Untracked
            } else if !remote.eq_ignore_ascii_case(&entry.checksum) {
                nimby_core::tracking::UpdateState::Outdated
            } else {
                nimby_core::tracking::UpdateState::UpToDate
            };
            let mut files: Vec<String> = Vec::new();

            // Compare the files on disk with the tracking index. The quick pass
            // only stats them (size + mtime); `--deep` hashes everything.
            if let Ok(local) = nimby_core::installed_manifest(&entry.target) {
                let result = index.check(&entry.target, &local, deep, Some(&cache));
                files = result.files.clone();
                if state == nimby_core::tracking::UpdateState::UpToDate && result.needs_work() {
                    state = result.state;
                }
            }

            if state != nimby_core::tracking::UpdateState::UpToDate {
                changed.push(json!({
                    "mod": entry.name,
                    "repo": repo,
                    "folder": folder.display().to_string(),
                    "state": state.as_str(),
                    "installed": entry.checksum,
                    "available": remote,
                    "files": files,
                }));
                wanted
                    .entry((repo.clone(), folder))
                    .or_default()
                    .push(entry.name.clone());
            }
        }
    }
    if let Ok(mut cache) = cache.lock() {
        let _ = cache.save();
    }

    if json_out {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "checked": installed_count,
                "deep": deep,
                "outdated": changed,
            }))?
        );
    } else if changed.is_empty() {
        println!(
            "everything is up to date ({} mods, {} check)",
            installed_count,
            if deep { "deep" } else { "quick" }
        );
    } else {
        for entry in &changed {
            let mut line = format!(
                "  {}  {} -> {}  [{}]",
                entry["mod"].as_str().unwrap_or_default(),
                entry["installed"].as_str().unwrap_or_default(),
                entry["available"].as_str().unwrap_or_default(),
                entry["state"].as_str().unwrap_or_default()
            );
            if let Some(files) = entry["files"].as_array() {
                if !files.is_empty() {
                    line.push_str(&format!(" {} file(s)", files.len()));
                }
            }
            println!("{line}");
        }
    }

    if check || wanted.is_empty() {
        return Ok(());
    }

    for ((repo, folder), names) in wanted {
        println!(
            "updating {} mods from {repo} in {}",
            names.len(),
            folder.display()
        );
        cmd_install(
            Some(&repo),
            Some(&folder),
            Some(&names.join(",")),
            None,
            false,
            None,
            false,
            false,
            true,
            Some(crate::ProgressArg::Text),
            None,
            None,
            OnError::Retry,
            Control::none(),
            3,
            false,
        )?;
    }
    Ok(())
}

pub fn cmd_repair(json_out: bool) -> Result<()> {
    let config = load_config()?;
    let cache_path = config::data_dir().join("hashcache.json");
    let cache = std::sync::Mutex::new(HashCache::load(&cache_path));

    let mut broken: Vec<(String, String, PathBuf)> = Vec::new();
    for (_, workspace) in config.workspaces() {
        for entry in installed_mods(&workspace)? {
            let folder = entry
                .target
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| workspace.clone());
            let Ok(manifest) = nimby_core::installed_manifest(&entry.target) else {
                broken.push((entry.repo, entry.name, folder));
                continue;
            };
            let report =
                nimby_core::verify::verify_dir_cached(&manifest, &entry.target, Some(&cache))?;
            if !report.is_ok() {
                broken.push((entry.repo, entry.name, folder));
            }
        }
    }
    if let Ok(mut cache) = cache.lock() {
        let _ = cache.save();
    }

    if json_out {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "broken": broken.iter().map(|(_, name, _)| name).collect::<Vec<_>>(),
            }))?
        );
    } else if broken.is_empty() {
        println!("nothing to repair");
    } else {
        println!("{} mod(s) need repair", broken.len());
    }
    if broken.is_empty() {
        return Ok(());
    }

    let mut groups: std::collections::BTreeMap<(String, PathBuf), Vec<String>> = Default::default();
    for (repo, name, folder) in broken {
        if repo.is_empty() {
            eprintln!("cannot repair {name}: no repository recorded");
            continue;
        }
        groups.entry((repo, folder)).or_default().push(name);
    }
    for ((repo, folder), names) in groups {
        cmd_install(
            Some(&repo),
            Some(&folder),
            Some(&names.join(",")),
            None,
            false,
            None,
            false,
            false,
            true,
            Some(crate::ProgressArg::Text),
            None,
            None,
            OnError::Retry,
            Control::none(),
            3,
            false,
        )?;
    }
    Ok(())
}

pub fn cmd_track(
    path: Option<&Path>,
    mod_name: Option<&str>,
    verify: bool,
    rebuild: bool,
    json_out: bool,
) -> Result<()> {
    let config = load_config()?;
    let store = config.resolved_shared();
    let workspace = path
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| config.active_workspace());

    let mut index = nimby_core::tracking::RepoIndex::load_result(&workspace)?;
    if rebuild {
        // Backfill from the lock files: no network, no hashing, exact provenance.
        let mut count = 0usize;
        for dir in nimby_core::workspace::mod_dirs(&workspace).unwrap_or_default() {
            let Some(lock) = nimby_core::read_lock(&dir) else {
                continue;
            };
            let Ok(manifest) = nimby_core::installed_manifest(&dir) else {
                continue;
            };
            index.rebuild_mod(&workspace, &manifest, &lock, store.as_deref());
            count += 1;
        }
        index.repo = config
            .active_repo()
            .map(|r| r.url.clone())
            .unwrap_or_default();
        index.save(&workspace)?;
        if !json_out {
            println!(
                "indexed {count} mod(s) into {}",
                nimby_core::tracking::tracking_path(&workspace).display()
            );
        }
    }

    if index.mods.is_empty() {
        if json_out {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "workspace": workspace.display().to_string(),
                    "tracked": 0,
                    "mods": [],
                }))?
            );
        } else {
            println!(
                "{}: no tracking data yet (run an install, or `nimby repair` to backfill)",
                workspace.display()
            );
        }
        return Ok(());
    }

    let mut rows = Vec::new();
    let mut problems = 0usize;
    for (name, track) in &index.mods {
        if let Some(wanted) = mod_name {
            if !wanted.eq_ignore_ascii_case(name) {
                continue;
            }
        }
        let mut totals = nimby_core::tracking::TrackTotals::default();
        for file in track.files.values() {
            totals.add(file);
        }
        let mut check = None;
        if verify {
            let target = workspace.join(name);
            match nimby_core::installed_manifest(&target) {
                Ok(manifest) => {
                    let result = index.check(&target, &manifest, true, None);
                    if result.needs_work() {
                        problems += 1;
                    }
                    check = Some(result);
                }
                Err(_) => problems += 1,
            }
        }
        rows.push(json!({
            "mod": name,
            "mode": track.mode,
            "checksum": track.checksum,
            "repo": track.repo,
            "installedAt": track.installed_at,
            "files": totals.files,
            "logicalBytes": totals.logical_bytes,
            "downloadedBytes": totals.downloaded_bytes,
            "workshopBytes": totals.workshop_bytes,
            "sharedBytes": totals.shared_bytes,
            "workspaceBytes": totals.workspace_bytes,
            "copiedBytes": totals.copied_bytes,
            "check": check,
        }));
    }

    if json_out {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "workspace": workspace.display().to_string(),
                "trackingFile": nimby_core::tracking::tracking_path(&workspace).display().to_string(),
                "updatedAt": index.updated_at,
                "repo": index.repo,
                "tracked": index.mods.len(),
                "problems": problems,
                "mods": rows,
            }))?
        );
        return Ok(());
    }

    println!("{}", workspace.display());
    println!(
        "tracking  {} · {} mods · updated {}",
        nimby_core::tracking::tracking_path(&workspace).display(),
        index.mods.len(),
        index.updated_at
    );
    println!(
        "{:<28} {:>6} {:>10} {:>10} {:>10} {:>10} {:>10}",
        "mod", "files", "logical", "downloaded", "workshop", "shared", "workspace"
    );
    for row in &rows {
        let size = |key: &str| format_size(row[key].as_u64().unwrap_or(0));
        println!(
            "{:<28} {:>6} {:>10} {:>10} {:>10} {:>10} {:>10}",
            truncate(row["mod"].as_str().unwrap_or_default(), 28),
            row["files"].as_u64().unwrap_or(0),
            size("logicalBytes"),
            size("downloadedBytes"),
            size("workshopBytes"),
            size("sharedBytes"),
            size("workspaceBytes"),
        );
        if !row["check"].is_null() {
            let check = &row["check"];
            if check["state"].as_str().unwrap_or("upToDate") != "upToDate" {
                println!(
                    "    needs work: {} ({} file(s))",
                    check["state"].as_str().unwrap_or(""),
                    check["files"].as_array().map(|f| f.len()).unwrap_or(0)
                );
            }
        }
    }
    if verify {
        if problems == 0 {
            println!("every tracked file matches its manifest");
        } else {
            println!("{problems} mod(s) need repair");
        }
    }
    Ok(())
}

pub fn cmd_store_info(json_out: bool) -> Result<()> {
    let config = load_config()?;
    let Some(path) = config.resolved_shared() else {
        anyhow::bail!("shared store is not configured; use `nimby config set --shared <path>`");
    };
    let stats = nimby_core::SharedStore::new(&path).stats()?;
    if json_out {
        println!("{}", serde_json::to_string_pretty(&stats)?);
    } else {
        println!(
            "{} · {} mods · {} files · {}",
            stats.path,
            stats.mods,
            stats.files,
            format_size(stats.bytes)
        );
    }
    Ok(())
}

pub fn cmd_store_prune(json_out: bool) -> Result<()> {
    let config = load_config()?;
    let Some(path) = config.resolved_shared() else {
        anyhow::bail!("shared store is not configured; use `nimby config set --shared <path>`");
    };
    let removed = nimby_core::SharedStore::new(&path).prune_empty()?;
    if json_out {
        println!("{}", json!({ "removed": removed }));
    } else {
        println!("removed {removed} empty store folder(s)");
    }
    Ok(())
}

pub fn cmd_migrate(source: Option<&Path>, dry_run: bool, json_out: bool) -> Result<()> {
    let settings = source
        .map(|p| p.to_path_buf())
        .unwrap_or_else(nimby_core::config::swifty_settings_path);
    if !settings.is_file() {
        anyhow::bail!("Swifty settings not found at {}", settings.display());
    }
    let text = fs::read_to_string(&settings)?;
    let root: Value = serde_json::from_str(nimby_core::util::strip_bom(&text))
        .with_context(|| format!("parsing {}", settings.display()))?;

    let mut config = load_config()?;
    let mut imported = json!({
        "source": settings.display().to_string(),
        "armaPath": null,
        "tempPath": null,
        "workspace": null,
        "params": [],
        "repos": [],
    });

    if let Some(arma) = root.get("armaPath").and_then(|v| v.as_str()) {
        let path = PathBuf::from(arma);
        if config::looks_like_arma(&path) {
            config.arma_path = Some(path.clone());
            imported["armaPath"] = json!(path.display().to_string());
        }
    }
    if let Some(temp) = root.get("tempPath").and_then(|v| v.as_str()) {
        let path = PathBuf::from(temp);
        config.temp_path = Some(path.clone());
        imported["tempPath"] = json!(path.display().to_string());
    }
    if let Some(parallel) = root.get("parallelDownloads").and_then(|v| v.as_u64()) {
        if parallel > 0 {
            config.parallel_downloads = parallel as u32;
        }
    }

    let mut params: Vec<nimby_core::LaunchParam> = Vec::new();
    let mut repos: Vec<Value> = Vec::new();
    if let Some(list) = root.get("repositories").and_then(|v| v.as_array()) {
        for repo in list {
            let url = repo.get("url").and_then(|v| v.as_str()).unwrap_or_default();
            if url.is_empty() {
                continue;
            }
            let name = repo
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let username = repo
                .get("username")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let password = repo
                .get("password")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let repo_path = repo
                .get("path")
                .and_then(|v| v.as_str())
                .filter(|value| !value.is_empty());
            if let Some(path) = repo_path {
                // Swifty's per-repo path becomes the workspace root; Nimby then
                // gives every repository its own subfolder under it.
                if imported["workspace"].is_null() {
                    config.workspace_path = PathBuf::from(path);
                    imported["workspace"] = json!(path);
                }
            }
            if params.is_empty() {
                if let Some(text) = repo.get("parameters").and_then(|v| v.as_str()) {
                    for token in text.split_whitespace() {
                        if token.starts_with('-') {
                            params.push(nimby_core::LaunchParam::parse(token));
                        }
                    }
                }
            }
            let index = config.upsert_repo(nimby_core::RepoEntry {
                url: url.to_string(),
                name: name.to_string(),
                username: username.to_string(),
                password: password.to_string(),
                last_checked: None,
                folder: None,
                path: None,
            });
            let folder = config.repos[index].folder.clone().unwrap_or_default();
            repos.push(json!({
                "url": nimby_core::config::normalize_url(url),
                "name": name,
                "folder": folder,
                "path": config.repos[index].workspace_under(&config.workspace_path).display().to_string(),
            }));
        }
    }
    if !params.is_empty() {
        for param in &params {
            if config
                .params
                .iter()
                .any(|existing| existing.key.eq_ignore_ascii_case(&param.key))
            {
                continue;
            }
            config.params.push(param.clone());
        }
        imported["params"] = json!(params.iter().map(|p| p.display()).collect::<Vec<_>>());
    }
    imported["repos"] = json!(repos);

    if !dry_run {
        config.save()?;
    }

    if json_out {
        println!("{}", serde_json::to_string_pretty(&imported)?);
    } else {
        println!("imported from {}", settings.display());
        println!("  arma      {}", imported["armaPath"]);
        println!("  temp      {}", imported["tempPath"]);
        println!("  workspace {}", imported["workspace"]);
        println!("  repos     {}", repos.len());
        println!("  params    {}", imported["params"]);
        if dry_run {
            println!("dry run: nothing written");
        }
    }
    Ok(())
}

/// Move mods that were installed straight into the workspace root into the
/// active repository's new subfolder. Renames only, so hardlinks survive.
pub fn cmd_migrate_layout(dry_run: bool, json_out: bool) -> Result<()> {
    let config = load_config()?;
    let root = config.workspace_root();
    let Some(repo) = config.active_repo() else {
        anyhow::bail!("no repository is selected; add or select one first");
    };
    let target = repo.workspace_under(&root);
    if !root.is_dir() {
        anyhow::bail!("workspace root {} does not exist", root.display());
    }

    let mut moved: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for entry in fs::read_dir(&root)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let is_mod = name.starts_with('@') && path.is_dir();
        let is_lock_dir = name == ".nimby" && path.is_dir();
        if !is_mod && !is_lock_dir {
            continue;
        }
        if path == target {
            continue;
        }
        let dest = target.join(&name);
        if dest.exists() {
            skipped.push(name);
            continue;
        }
        if !dry_run {
            fs::create_dir_all(&target)?;
            if let Err(err) = fs::rename(&path, &dest) {
                anyhow::bail!(
                    "moving {} to {} failed: {err} (move it manually, then re-run)",
                    path.display(),
                    dest.display()
                );
            }
        }
        moved.push(name);
    }

    if json_out {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "root": root.display().to_string(),
                "target": target.display().to_string(),
                "dryRun": dry_run,
                "moved": moved,
                "skipped": skipped,
            }))?
        );
    } else if moved.is_empty() && skipped.is_empty() {
        println!(
            "nothing to move: {} already holds no loose mods",
            root.display()
        );
    } else {
        for name in &moved {
            println!(
                "{} {name} -> {}",
                if dry_run { "would move" } else { "moved" },
                target.display()
            );
        }
        for name in &skipped {
            println!("skipped {name}: already exists in {}", target.display());
        }
        if dry_run {
            println!("dry run: nothing was moved");
        }
    }
    Ok(())
}

pub fn cmd_status(path: Option<&Path>, json_out: bool) -> Result<()> {
    let config = load_config()?;
    // Status is about one install folder: the explicit one, else the active
    // repository's own folder.
    let workspace = path
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| config.active_workspace());

    let mut mods = Vec::new();
    if workspace.is_dir() {
        for dir in nimby_core::workspace::mod_dirs(&workspace)? {
            let name = dir
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let lock = nimby_core::read_lock(&dir);
            // `partial` is the marker written before an install starts; it means
            // the last run did not finish.
            let mode = lock.as_ref().map(|l| {
                if l.mode == "partial" {
                    "incomplete".to_string()
                } else {
                    l.mode.clone()
                }
            });
            mods.push(json!({
                "mod": name,
                "checksum": lock.as_ref().map(|l| l.checksum.clone()),
                "mode": mode,
                "incomplete": lock.as_ref().map(|l| l.mode == "partial").unwrap_or(false),
                "repo": lock.as_ref().and_then(|l| l.repo.clone()),
                "kind": if linker::is_link(&dir) { "link" } else { "real" },
                "logicalBytes": lock.as_ref().map(|l| l.stats.logical_bytes).unwrap_or(0),
                "savedBytes": lock.as_ref().map(|l| l.stats.workshop_bytes + l.stats.store_bytes).unwrap_or(0),
            }));
        }
    }

    if json_out {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "workspace": workspace.display().to_string(),
                "mods": mods,
            }))?
        );
        return Ok(());
    }

    println!("{}", workspace.display());
    if mods.is_empty() {
        println!("  (empty)");
    }
    for row in &mods {
        println!(
            "  {:<32} {:<10} {:<6} {}",
            row["mod"].as_str().unwrap_or_default(),
            row["mode"].as_str().unwrap_or("-"),
            row["kind"].as_str().unwrap_or_default(),
            row["checksum"].as_str().unwrap_or_default(),
        );
    }
    Ok(())
}

pub fn cmd_stats(path: Option<&Path>, json_out: bool) -> Result<()> {
    let config = load_config()?;
    // Without an explicit folder, every repository folder is reported, so the
    // numbers always match the repo list.
    let workspaces: Vec<PathBuf> = match path {
        Some(p) => vec![p.to_path_buf()],
        None => config
            .workspaces()
            .into_iter()
            .map(|(_, path)| path)
            .collect(),
    };
    let store = config.resolved_shared();
    let report = stats::collect_many(&workspaces, store.as_deref())?;

    if json_out {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    println!("workspace {}", report.workspace);
    if let Some(store) = &report.store {
        println!(
            "store     {} · {} mods · {} files · {}",
            store.path,
            store.mods,
            store.files,
            format_size(store.bytes)
        );
    } else {
        println!("store     (disabled)");
    }
    if report.repos.is_empty() {
        println!("no installed mods");
        return Ok(());
    }
    println!();
    println!(
        "{:<40} {:>5} {:>12} {:>12} {:>12} {:>12}",
        "repo", "mods", "logical", "on disk", "in workspace", "saved"
    );
    for repo in &report.repos {
        println!(
            "{:<40} {:>5} {:>12} {:>12} {:>12} {:>12}",
            truncate(&repo.repo, 40),
            repo.mods,
            format_size(repo.logical_bytes),
            format_size(repo.on_disk_bytes()),
            format_size(repo.workspace_bytes()),
            format_size(repo.saved_bytes()),
        );
    }
    println!(
        "{:<40} {:>5} {:>12} {:>12} {:>12} {:>12}",
        "total",
        report.totals.mods,
        format_size(report.totals.logical_bytes),
        format_size(report.totals.on_disk_bytes()),
        format_size(report.totals.workspace_bytes()),
        format_size(report.totals.saved_bytes()),
    );
    println!(
        "on disk = shared store + workspace copies · saved = bytes linked (Workshop or store) instead of copied"
    );
    Ok(())
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_string()
    } else {
        let mut out: String = value.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

pub fn cmd_doctor(json_out: bool) -> Result<()> {
    let config = load_config().unwrap_or_default();
    let steam_root = steam::steam_root();
    let libraries = steam::library_folders();
    let arma = config.resolved_arma();
    let workshop = steam::scan_workshop(steam::ARMA3_APP_ID);

    let probe_dir = tempfile::tempdir()?;
    let target = probe_dir.path().join("target");
    fs::create_dir(&target)?;
    let link = probe_dir.path().join("link");
    let dir_link = linker::link_dir(&link, &target).is_ok();
    let hard = {
        let file = probe_dir.path().join("a.bin");
        fs::write(&file, b"x")?;
        let hard = probe_dir.path().join("b.bin");
        fs::hard_link(&file, &hard).is_ok()
    };
    let symlink = {
        let target2 = probe_dir.path().join("t2");
        fs::create_dir(&target2)?;
        std::os::windows::fs::symlink_dir(&target2, probe_dir.path().join("s2")).is_ok()
    };
    let running = launch::running_processes();
    let store = config
        .resolved_shared()
        .and_then(|path| nimby_core::SharedStore::new(path).stats().ok());

    if json_out {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "steamRoot": steam_root.map(|p| p.display().to_string()),
                "libraries": libraries.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
                "armaPath": arma.map(|p| p.display().to_string()),
                "armaSource": if config.arma_path.is_some() { "config" } else { "autodetect" },
                "workshopItems": workshop.len(),
                "dirLink": dir_link,
                "hardlink": hard,
                "symlink": symlink,
                "running": running,
                "store": store,
                "tempPath": config.resolved_temp().display().to_string(),
                "configPath": config::config_path().display().to_string(),
            }))?
        );
        return Ok(());
    }

    println!(
        "steam   {}",
        steam_root
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "not found".into())
    );
    println!("libraries {}", libraries.len());
    println!(
        "arma    {}",
        arma.map(|p| p.display().to_string())
            .unwrap_or_else(|| "not found".into())
    );
    println!("workshop {} items", workshop.len());
    println!("links   dir={dir_link} hardlink={hard} symlink={symlink}");
    match &store {
        Some(store) => println!(
            "store   {} · {} mods · {}",
            store.path,
            store.mods,
            format_size(store.bytes)
        ),
        None => println!("store   (disabled)"),
    }
    println!("temp    {}", config.resolved_temp().display());
    if !running.is_empty() {
        println!("running {}", running.join(", "));
    }
    // Attribution + GPL source offer: shown by the diagnostic command so it is
    // always one command away.
    println!(
        "nimby   {} · GPL-3.0-or-later · © 2026 ffredyk",
        env!("CARGO_PKG_VERSION")
    );
    println!("        https://nimby.ffredyk.cz · https://github.com/ffredyk/nimby");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn cmd_launch(
    arma: Option<&Path>,
    path: Option<&Path>,
    repo: Option<&str>,
    server: Option<&str>,
    battleye: bool,
    extra: &[String],
    dry_run: bool,
    json_out: bool,
) -> Result<()> {
    let config = load_config()?;

    let arma_dir = match arma {
        Some(dir) => dir.to_path_buf(),
        None => config.resolved_arma().ok_or_else(|| {
            anyhow::anyhow!("Arma 3 not found; set it with `nimby config set --arma <path>`")
        })?,
    };
    let workspace = path.map(|p| p.to_path_buf()).unwrap_or_else(|| match repo {
        Some(url) => config.workspace_for(url),
        None => config.active_workspace(),
    });

    let mod_paths: Vec<PathBuf> = nimby_core::workspace::mod_dirs(&workspace)?
        .into_iter()
        .collect();

    // Parameters: configured ones first, then repo defaults, then CLI extras.
    let mut rendered = params::render_all(&config.params);
    let mut connect = None;
    let mut port = None;
    let mut password = None;
    let mut servers = Vec::new();

    let mut repo_url = repo.map(config::normalize_url);
    if repo_url.is_none() {
        repo_url = config.active_repo().map(|r| r.url.clone());
    }
    if let Some(url) = &repo_url {
        if let Ok(http) = crate::publish::repo_http(url) {
            if let Ok(text) = http.get("repo.json") {
                if let Ok(manifest) = nimby_core::RepoManifest::parse(&text) {
                    rendered.extend(launch::split_params(&manifest.client_parameters));
                    for entry in &manifest.servers {
                        servers.push(entry.name.clone());
                    }
                    if let Some(name) = server {
                        match manifest
                            .servers
                            .iter()
                            .find(|s| s.name.eq_ignore_ascii_case(name))
                        {
                            Some(entry) => {
                                connect = Some(entry.address.clone());
                                port = entry.port.parse().ok();
                                password = if entry.password.is_empty() {
                                    None
                                } else {
                                    Some(entry.password.clone())
                                };
                            }
                            None => anyhow::bail!("server '{name}' not found in repo.json"),
                        }
                    }
                }
            }
        }
    }
    for raw in extra {
        rendered.extend(launch::split_params(raw));
    }
    // A `-password=` or `-connect=` typed by the user wins over the repo value.
    for param in &rendered {
        if let Some(value) = param.strip_prefix("-connect=") {
            connect = Some(value.to_string());
        } else if let Some(value) = param.strip_prefix("-port=") {
            port = value.parse().ok();
        }
    }
    rendered.retain(|p| !p.starts_with("-mod="));
    // Repo defaults and user parameters often overlap; keep the first one.
    let mut seen = std::collections::HashSet::new();
    rendered.retain(|p| seen.insert(p.clone()));

    let opts = LaunchOptions {
        arma_dir,
        mod_paths,
        connect,
        port,
        password,
        battleye: battleye || config.battleye,
        extra_params: rendered,
    };
    let line = launch::command_line(&opts, true)?;

    if dry_run {
        if json_out {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "commandLine": line,
                    "servers": servers,
                    "mods": opts.mod_paths.len(),
                }))?
            );
        } else {
            println!("{line}");
        }
        return Ok(());
    }

    if launch::is_arma_running() {
        anyhow::bail!("Arma 3 is already running; close it first");
    }
    let mut child = launch::spawn(&opts)?;
    if json_out {
        println!("{}", json!({ "commandLine": line, "pid": child.id() }));
    } else {
        println!("launched pid {}", child.id());
    }
    let _ = child.wait();
    Ok(())
}

/// Ensure a path exists, used by tests and the GUI through the CLI.
#[allow(dead_code)]
fn ensure(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::truncate;

    #[test]
    fn truncates_long_repo_names() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("a-very-long-repository-url", 10), "a-very-lo…");
    }
}
