use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::checksum::md5_hex;
use crate::error::{Error, Result};
use crate::formats::{ModFile, ModManifest, SrfFormat};
use crate::http::HttpSource;
use crate::linker::{self, LinkKind, LinkMode};
use crate::store::SharedStore;
use crate::util::join_rel;
use crate::verify::{check_file, check_file_shared, verify_dir_cached, FileStatus};
use crate::workspace::{write_lock_full, LockStats};

pub struct InstallOptions<'a> {
    pub mode: LinkMode,
    /// Workshop item roots to borrow files from (already discovered by caller).
    pub workshop_roots: &'a [PathBuf],
    /// Shared store root. Downloaded files are written here once and linked.
    pub store_root: Option<&'a Path>,
    pub http: Option<&'a HttpSource>,
    /// Optional local mirror of the repo (offline installs).
    pub local_repo: Option<&'a Path>,
    /// Repository URL recorded in the lock file, for stats.
    pub repo: Option<&'a str>,
    pub force: bool,
    /// Called for every file the installer materializes, so callers can show
    /// accurate progress over the total byte count.
    pub on_progress: Option<&'a ProgressFn<'a>>,
    /// Maximum number of concurrent file downloads. 1 = sequential.
    ///
    /// The installer starts here and lowers the count when disk writes dominate
    /// (so a slow disk is not hammered), raising it again when the link is the
    /// bottleneck.
    pub max_parallel: usize,
    /// Persistent hash cache so unchanged Workshop files are not rehashed.
    pub cache: Option<&'a std::sync::Mutex<crate::hashcache::HashCache>>,
    /// Cooperative cancellation. Checked between files, between download waves
    /// and inside a running transfer, so a cancel is quick but never leaves a
    /// file half-written.
    pub cancel: Option<&'a std::sync::atomic::AtomicBool>,
    /// How many times a single file transfer is attempted before the error is
    /// reported to the caller.
    pub retries: u32,
}

/// Callback type for install progress.
pub type ProgressFn<'a> = dyn for<'p> Fn(ProgressEvent<'p>) + Send + Sync + 'a;

/// Where a file's bytes came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileAction {
    /// Hardlinked/symlinked from the Steam Workshop cache.
    Workshop,
    /// Hardlinked/symlinked from the shared store.
    Store,
    /// Fetched over the network.
    Download,
    /// Copied, so the file really occupies space in the workspace.
    Copy,
    /// Already present and byte-identical from an interrupted install.
    Reuse,
}

impl Default for InstallOptions<'_> {
    fn default() -> Self {
        Self {
            mode: LinkMode::Auto,
            workshop_roots: &[],
            store_root: None,
            http: None,
            local_repo: None,
            repo: None,
            force: false,
            on_progress: None,
            max_parallel: 4,
            cache: None,
            cancel: None,
            retries: 3,
        }
    }
}

/// One progress step. Borrowed strings keep the hot path allocation free.
#[derive(Clone, Copy, Debug)]
pub enum ProgressEvent<'a> {
    /// A file is about to be checked, linked or downloaded.
    FileStart {
        mod_name: &'a str,
        path: &'a str,
        bytes: u64,
    },
    /// Bytes of the current file have arrived (or been hashed) so far.
    FileBytes {
        mod_name: &'a str,
        path: &'a str,
        done: u64,
        total: u64,
    },
    /// A single file was materialized.
    File {
        mod_name: &'a str,
        path: &'a str,
        action: FileAction,
        bytes: u64,
    },
    /// The whole mod folder was linked in one step (dir-link or passthrough).
    ModLinked {
        mod_name: &'a str,
        action: FileAction,
        bytes: u64,
    },
    /// Informational message, for example a concurrency change.
    Note { message: &'a str },
    /// A transfer failed and is being retried after a short pause.
    Retry {
        mod_name: &'a str,
        path: &'a str,
        attempt: u32,
        attempts: u32,
        message: &'a str,
    },
    /// Files that were already in place from an earlier, interrupted run were
    /// verified and kept instead of being fetched again.
    Resumed {
        mod_name: &'a str,
        files: usize,
        bytes: u64,
    },
}

/// Timing for one fetched file, used to tune download concurrency.
#[derive(Clone, Copy, Debug, Default)]
struct Fetched {
    bytes: u64,
    /// `true` when the bytes came over the network (not from a local mirror).
    network: bool,
    /// `true` when nothing had to be transferred because a complete payload
    /// from an earlier run was still in the store.
    existing: bool,
    elapsed_nanos: u64,
    write_nanos: u64,
}

/// Count bytes that an earlier run already materialized correctly.
///
/// Provenance comes from the previous lock, so an interrupted install does not
/// turn Workshop bytes into "downloaded" bytes in the savings stats.
fn account_kept(origin: &str, bytes: u64, report: &mut InstallReport, stats: &mut LockStats) {
    report.resumed_files += 1;
    report.resumed_bytes += bytes;
    if origin.starts_with("workshop:") {
        report.reused_bytes += bytes;
        stats.workshop_bytes += bytes;
    } else if origin.starts_with("store:") {
        report.store_files += 1;
        report.store_bytes += bytes;
        stats.store_bytes += bytes;
    } else if origin.starts_with("copy:") {
        stats.copied_bytes += bytes;
    } else {
        // Unknown or downloaded earlier: the bytes are on disk either way.
        stats.downloaded_bytes += bytes;
    }
}

/// Raise or lower the worker count from disk-vs-network timing.
///
/// `write_share` is the fraction of wall time spent writing to disk. High share
/// means the disk is the bottleneck, so fewer workers help; low share means the
/// network is the bottleneck and more workers help.
fn adjust_parallel(limit: usize, max: usize, write_share: f64, streak: &mut i32) -> usize {
    if max <= 1 {
        return 1;
    }
    let direction = if write_share > 0.55 {
        -1
    } else if write_share < 0.2 {
        1
    } else {
        0
    };
    if direction == 0 {
        *streak = 0;
        return limit;
    }
    if *streak != 0 && (*streak > 0) != (direction > 0) {
        *streak = 0;
    }
    *streak += direction;
    if streak.abs() < 2 {
        return limit;
    }
    *streak = 0;
    if direction > 0 {
        (limit + 1).min(max)
    } else {
        limit.saturating_sub(1).max(1)
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallReport {
    pub mod_name: String,
    pub target: String,
    pub mode: String,
    pub linked_files: usize,
    pub copied_files: usize,
    pub downloaded_files: usize,
    /// Files reused from the shared store without any network traffic.
    pub store_files: usize,
    /// Bytes linked from the Workshop cache.
    pub reused_bytes: u64,
    /// Bytes linked from the shared store.
    pub store_bytes: u64,
    /// Bytes actually transferred.
    pub downloaded_bytes: u64,
    /// Files kept from an interrupted install because they already matched.
    pub resumed_files: usize,
    pub resumed_bytes: u64,
    /// Files removed because the manifest no longer lists them.
    pub pruned_files: usize,
    /// Where every file ended up: the source of the tracking index.
    pub files: Vec<InstalledFile>,
    pub verified: bool,
}

/// One file the installer materialized, with its provenance.
///
/// This is what the tracking index records, so the Stats page and `update` can
/// tell downloaded bytes from Workshop and shared-store bytes without hashing
/// anything again.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledFile {
    pub path: String,
    pub bytes: u64,
    pub checksum: String,
    pub origin: crate::tracking::FileOrigin,
    pub location: crate::tracking::FileLocation,
    /// `hardlink`, `symlink`, `junction`, `copy` or `existing`.
    pub link: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

fn link_label(kind: LinkKind) -> &'static str {
    match kind {
        LinkKind::Existing => "existing",
        LinkKind::Junction => "junction",
        LinkKind::Symlink => "symlink",
        LinkKind::Hardlink => "hardlink",
        LinkKind::Copy => "copy",
    }
}

/// Where a reused file lives, inferred from the previous lock entry.
///
/// Downloads go into the shared store when one is configured, otherwise into the
/// workspace; Workshop and store entries say so directly.
fn location_for(lock_entry: &str, has_store: bool) -> crate::tracking::FileLocation {
    match lock_entry.split_once(':').map(|(kind, _)| kind) {
        Some("workshop") => crate::tracking::FileLocation::Workshop,
        Some("store") => crate::tracking::FileLocation::Store,
        _ if has_store => crate::tracking::FileLocation::Store,
        _ => crate::tracking::FileLocation::Workspace,
    }
}

/// Tracking entries for a mod that was linked as a whole folder.
fn whole_mod_files(
    manifest: &ModManifest,
    origin: Origin,
    root: &Path,
    link: &str,
) -> Vec<InstalledFile> {
    let (file_origin, location) = match origin {
        Origin::Workshop => (
            crate::tracking::FileOrigin::Workshop,
            crate::tracking::FileLocation::Workshop,
        ),
        Origin::Store => (
            crate::tracking::FileOrigin::Store,
            crate::tracking::FileLocation::Store,
        ),
    };
    manifest
        .files
        .iter()
        .map(|file| InstalledFile {
            path: file.path.clone(),
            bytes: file.length,
            checksum: file.checksum.clone(),
            origin: file_origin,
            location,
            link: link.to_string(),
            source: Some(root.display().to_string()),
        })
        .collect()
}

impl InstallReport {
    /// Bytes that did not need to be written into the workspace.
    pub fn saved_bytes(&self) -> u64 {
        self.reused_bytes + self.store_bytes
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Origin {
    Workshop,
    Store,
}

fn is_managed(target: &Path) -> bool {
    crate::workspace::is_managed(target)
}

fn emit(opts: &InstallOptions, event: ProgressEvent<'_>) {
    if let Some(callback) = opts.on_progress {
        callback(event);
    }
}

fn cancelled(opts: &InstallOptions) -> bool {
    opts.cancel
        .map(|flag| flag.load(std::sync::atomic::Ordering::Relaxed))
        .unwrap_or(false)
}

/// Bail out between files, so a cancel never depends on the network finishing.
fn check_cancel(opts: &InstallOptions) -> Result<()> {
    if cancelled(opts) {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}

/// `true` when at least one manifest file already exists under `target`.
///
/// Used to decide whether a failed install may be deleted: an empty shell is
/// noise, but a folder that already holds verified payload is exactly what makes
/// a retry cheap.
fn target_has_payload(target: &Path, manifest: &ModManifest) -> bool {
    manifest
        .files
        .iter()
        .filter_map(|file| join_rel(target, &file.path).ok())
        .any(|path| path.is_file())
}

/// Delete files under `target` that the manifest does not list, then drop the
/// directories that became empty. Only used when an existing folder is reused.
fn prune_strays(target: &Path, manifest: &ModManifest) -> Result<usize> {
    use std::collections::BTreeSet;
    let keep: BTreeSet<&str> = manifest.files.iter().map(|f| f.path.as_str()).collect();
    let mut removed = 0usize;
    let mut dirs: Vec<PathBuf> = Vec::new();

    fn walk(
        root: &Path,
        dir: &Path,
        keep: &BTreeSet<&str>,
        removed: &mut usize,
        dirs: &mut Vec<PathBuf>,
    ) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.is_dir() {
                dirs.push(path.clone());
                walk(root, &path, keep, removed, dirs);
                continue;
            }
            let Some(rel) = crate::util::rel_path(root, &path) else {
                continue;
            };
            if rel == crate::formats::SRF_FILE_NAME || keep.contains(rel.as_str()) {
                continue;
            }
            if linker::remove_path(&path).is_ok() {
                *removed += 1;
            }
        }
    }

    walk(target, target, &keep, &mut removed, &mut dirs);
    // Deepest first so a directory emptied by the loop above disappears too.
    dirs.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    for dir in dirs {
        let empty = fs::read_dir(&dir)
            .map(|mut e| e.next().is_none())
            .unwrap_or(false);
        if empty {
            let _ = fs::remove_dir(&dir);
        }
    }
    Ok(removed)
}

/// Find a root that contains every file byte for byte.
fn find_complete_root(
    manifest: &ModManifest,
    roots: &[(PathBuf, Origin)],
    cache: Option<&std::sync::Mutex<crate::hashcache::HashCache>>,
) -> Option<(PathBuf, Origin)> {
    use rayon::prelude::*;
    for (root, origin) in roots {
        let complete = manifest.files.par_iter().all(|file| {
            let Ok(path) = join_rel(root, &file.path) else {
                return false;
            };
            check_file_shared(file, &path, cache) == FileStatus::Ok
        });
        if complete {
            return Some((root.clone(), *origin));
        }
    }
    None
}

fn matches_for_file(
    file: &ModFile,
    roots: &[(PathBuf, Origin)],
    cache: Option<&std::sync::Mutex<crate::hashcache::HashCache>>,
) -> Option<(PathBuf, Origin)> {
    for (root, origin) in roots {
        let Ok(path) = join_rel(root, &file.path) else {
            continue;
        };
        if check_file_shared(file, &path, cache) == FileStatus::Ok {
            return Some((path, *origin));
        }
    }
    None
}

/// Range request size. PBOs have thousands of parts; asking for one part at a
/// time costs a round trip each (hundreds of seconds on high-latency links), so
/// contiguous parts are merged into chunks of this size.
const CHUNK_BYTES: u64 = 8 << 20;

/// A contiguous byte range of a file, covering one or more parts.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Chunk {
    start: u64,
    len: u64,
    /// Part indices `[first, last]`.
    first_part: usize,
    last_part: usize,
}

/// Merge contiguous parts into chunks of at most `chunk_bytes`.
fn plan_chunks(parts: &[crate::formats::FilePart], chunk_bytes: u64) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    let mut index = 0usize;
    while index < parts.len() {
        let start = parts[index].start;
        let first = index;
        let mut len = 0u64;
        while index < parts.len()
            && len + parts[index].length <= chunk_bytes
            && parts[index].start == start + len
        {
            len += parts[index].length;
            index += 1;
        }
        if len == 0 {
            // A single part larger than the chunk size: take it as its own chunk.
            len = parts[index].length;
            index += 1;
        }
        chunks.push(Chunk {
            start,
            len,
            first_part: first,
            last_part: index - 1,
        });
    }
    chunks
}

fn download_file(
    http: &HttpSource,
    rel: &str,
    file: &ModFile,
    dest: &Path,
    cancel: Option<&std::sync::atomic::AtomicBool>,
    on_bytes: &dyn Fn(u64),
) -> Result<(u64, u64)> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let stopped = || {
        cancel
            .map(|flag| flag.load(std::sync::atomic::Ordering::Relaxed))
            .unwrap_or(false)
    };

    // Resume: keep a complete-looking temporary file and re-use the parts that
    // still hash correctly.
    let mut resumed_parts = vec![false; file.parts.len()];
    let mut resumed_bytes = 0u64;
    let mut resumable = false;
    if let Ok(meta) = fs::metadata(dest) {
        if meta.len() == file.length {
            resumable = true;
            for (index, part) in file.parts.iter().enumerate() {
                if crate::checksum::md5_range(dest, part.start, part.length)
                    .map(|hash| hash.eq_ignore_ascii_case(&part.checksum))
                    .unwrap_or(false)
                {
                    resumed_parts[index] = true;
                    resumed_bytes += part.length;
                }
            }
        }
    }
    if resumed_bytes > 0 {
        on_bytes(resumed_bytes);
    }

    // Establish the final length up front so every chunk can write at its own
    // offset. Each worker opens its own handle: a cloned handle shares the file
    // pointer on Windows, and concurrent seek+write pairs would corrupt the file.
    if !resumable {
        let handle = File::create(dest)?;
        handle.set_len(file.length)?;
    }

    let chunks = plan_chunks(&file.parts, CHUNK_BYTES);
    // Chunks that are already complete from a previous attempt are skipped.
    let pending: Vec<Chunk> = chunks
        .into_iter()
        .filter(|chunk| !(chunk.first_part..=chunk.last_part).all(|index| resumed_parts[index]))
        .collect();

    let workers = pending.len().clamp(1, 4);
    let written = std::sync::atomic::AtomicU64::new(resumed_bytes);
    let mut network = 0u64;
    let mut write_nanos = 0u64;
    let mut ranged_supported = true;

    let mut index = 0usize;
    while index < pending.len() && ranged_supported {
        if stopped() {
            return Err(Error::Cancelled);
        }
        let batch = &pending[index..pending.len().min(index + workers)];
        let results: Vec<Result<(u64, u64, bool)>> = std::thread::scope(|scope| {
            let handles: Vec<_> = batch
                .iter()
                .map(|chunk| {
                    scope.spawn(move || {
                        let Some(bytes) = http.get_range(rel, chunk.start, chunk.len)? else {
                            return Ok((0u64, 0u64, false));
                        };
                        if bytes.len() as u64 != chunk.len {
                            return Err(Error::msg(format!(
                                "{rel}: short range response ({} != {})",
                                bytes.len(),
                                chunk.len
                            )));
                        }
                        for part_index in chunk.first_part..=chunk.last_part {
                            let part = &file.parts[part_index];
                            let offset = (part.start - chunk.start) as usize;
                            let slice = bytes
                                .get(offset..offset + part.length as usize)
                                .ok_or_else(|| Error::msg(format!("{rel}: part out of range")))?;
                            let actual = md5_hex(slice);
                            if !actual.eq_ignore_ascii_case(&part.checksum) {
                                return Err(Error::HashMismatch {
                                    path: format!("{rel}:{}", part.path),
                                    expected: part.checksum.clone(),
                                    actual,
                                });
                            }
                        }
                        let mut handle =
                            fs::OpenOptions::new().read(true).write(true).open(dest)?;
                        handle.seek(SeekFrom::Start(chunk.start))?;
                        let started = std::time::Instant::now();
                        handle.write_all(&bytes)?;
                        let nanos = started.elapsed().as_nanos() as u64;
                        Ok((chunk.len, nanos, true))
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| match handle.join() {
                    Ok(result) => result,
                    Err(_) => Err(Error::msg("download worker panicked")),
                })
                .collect()
        });

        let mut batch_written = 0u64;
        for result in results {
            let (len, nanos, ranged) = result?;
            if !ranged {
                ranged_supported = false;
                break;
            }
            network += len;
            batch_written += len;
            write_nanos += nanos;
        }
        if batch_written > 0 {
            let total = written.fetch_add(batch_written, std::sync::atomic::Ordering::Relaxed)
                + batch_written;
            on_bytes(total);
        }
        index += batch.len();
    }

    if ranged_supported {
        if check_file(file, dest) != FileStatus::Ok {
            return Err(Error::HashMismatch {
                path: rel.to_string(),
                expected: file.checksum.clone(),
                actual: String::from("range assembly"),
            });
        }
        return Ok((network, write_nanos));
    }

    // No Range support: stream the whole body so the caller still sees byte
    // progress instead of a single silent transfer.
    let mut reader = http.get_reader(rel)?;
    let mut out = File::create(dest)?;
    let mut buffer = vec![0u8; 1 << 20];
    let mut streamed = 0u64;
    let mut write_nanos = 0u64;
    let step = std::cmp::max(1 << 20, file.length / 50);
    let mut last = 0u64;
    loop {
        if stopped() {
            return Err(Error::Cancelled);
        }
        let read = std::io::Read::read(&mut reader, &mut buffer)?;
        if read == 0 {
            break;
        }
        let started = std::time::Instant::now();
        out.write_all(&buffer[..read])?;
        write_nanos += started.elapsed().as_nanos() as u64;
        streamed += read as u64;
        if streamed.saturating_sub(last) >= step || streamed == file.length {
            last = streamed;
            on_bytes(streamed);
        }
    }
    out.flush()?;
    drop(out);

    if streamed != file.length {
        return Err(Error::msg(format!(
            "{rel}: expected {} bytes, got {}",
            file.length, streamed
        )));
    }
    if check_file(file, dest) != FileStatus::Ok {
        return Err(Error::HashMismatch {
            path: rel.to_string(),
            expected: file.checksum.clone(),
            actual: String::from("streamed body"),
        });
    }
    Ok((streamed, write_nanos))
}

fn copy_from_local(local: &Path, rel: &str, file: &ModFile, dest: &Path) -> Result<u64> {
    let src = join_rel(local, rel)?;
    if check_file(file, &src) != FileStatus::Ok {
        return Err(Error::msg(format!(
            "{} is not the expected content",
            src.display()
        )));
    }
    linker::copy_file(&src, dest)?;
    Ok(file.length)
}

/// Put `file` at `payload_dest` by downloading or copying from a local mirror.
fn fetch_payload(
    manifest: &ModManifest,
    file: &ModFile,
    payload_dest: &Path,
    opts: &InstallOptions,
    on_bytes: &dyn Fn(u64),
) -> Result<Fetched> {
    let rel = format!("{}/{}", manifest.name, file.path);
    if let Some(http) = opts.http {
        // Write through a temporary file so a failed transfer never leaves a
        // half-written payload behind. The name keeps the original file name, so
        // `a.pbo` and `a.bisign` cannot collide.
        let temp = match payload_dest.file_name() {
            Some(name) => {
                payload_dest.with_file_name(format!("{}.nimby-tmp", name.to_string_lossy()))
            }
            None => payload_dest.with_extension("nimby-tmp"),
        };
        let started = std::time::Instant::now();
        // A transient failure is retried here: the temporary file survives, so
        // every attempt verifies the parts it already holds and resumes.
        let mut notify = |notice: crate::retry::RetryNotice<'_>| {
            let message = notice.error.to_string();
            emit(
                opts,
                ProgressEvent::Retry {
                    mod_name: &manifest.name,
                    path: &file.path,
                    attempt: notice.attempt,
                    attempts: notice.attempts,
                    message: &message,
                },
            );
        };
        let policy = crate::retry::RetryPolicy::with_attempts(opts.retries.max(1));
        match crate::retry::with_retry(policy, &mut notify, |_| {
            download_file(http, &rel, file, &temp, opts.cancel, on_bytes)
        }) {
            Ok((bytes, write_nanos)) => {
                let elapsed_nanos = started.elapsed().as_nanos() as u64;
                if let Some(parent) = payload_dest.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::rename(&temp, payload_dest)?;
                Ok(Fetched {
                    bytes,
                    network: true,
                    existing: false,
                    elapsed_nanos,
                    write_nanos,
                })
            }
            Err(err) => {
                // Keep the partial file: the next attempt verifies which parts
                // already match and resumes instead of starting over.
                Err(err)
            }
        }
    } else if let Some(local) = opts.local_repo {
        let started = std::time::Instant::now();
        copy_from_local(local, &rel, file, payload_dest)?;
        let elapsed_nanos = started.elapsed().as_nanos() as u64;
        on_bytes(file.length);
        Ok(Fetched {
            bytes: file.length,
            network: false,
            existing: false,
            elapsed_nanos,
            write_nanos: elapsed_nanos,
        })
    } else {
        Err(Error::MissingFile(PathBuf::from(&file.path)))
    }
}

fn place(dest: &Path, src: &Path, copy_only: bool) -> Result<LinkKind> {
    if copy_only {
        linker::copy_file(src, dest)?;
        return Ok(LinkKind::Copy);
    }
    linker::link_file(dest, src)
}

/// Install one mod. An existing folder that Nimby does not own is never
/// touched. A failed or cancelled install keeps whatever it already verified so
/// the next run resumes instead of starting over — unless nothing at all was
/// placed, in which case the empty shell is removed.
pub fn install_mod(
    manifest: &ModManifest,
    target: &Path,
    opts: &InstallOptions,
) -> Result<InstallReport> {
    if linker::exists(target) && !(opts.force || is_managed(target) || linker::is_link(target)) {
        return Err(Error::msg(format!(
            "{} exists and is not managed by Nimby; pass --force to replace it",
            target.display()
        )));
    }
    let result = install_mod_inner(manifest, target, opts);
    if let Err(err) = &result {
        if !target_has_payload(target, manifest) {
            let _ = linker::remove_path(target);
        } else {
            emit(
                opts,
                ProgressEvent::Note {
                    message: if err.is_cancelled() {
                        "partial install kept; the next run resumes from it"
                    } else {
                        "partial install kept so the retry can resume"
                    },
                },
            );
        }
    }
    result
}

fn install_mod_inner(
    manifest: &ModManifest,
    target: &Path,
    opts: &InstallOptions,
) -> Result<InstallReport> {
    check_cancel(opts)?;
    let existed = linker::exists(target);
    if existed && !(opts.force || is_managed(target) || linker::is_link(target)) {
        return Err(Error::msg(format!(
            "{} exists and is not managed by Nimby; pass --force to replace it",
            target.display()
        )));
    }
    // Reuse what an interrupted run left behind. The previous lock tells us
    // where the kept bytes came from, so the savings stats stay honest.
    let resume = existed && !opts.force && is_managed(target);
    let previous = if resume {
        crate::workspace::read_lock(target)
    } else {
        None
    };
    if existed && !resume {
        linker::remove_path(target)?;
    }

    let mut report = InstallReport {
        mod_name: manifest.name.clone(),
        target: target.display().to_string(),
        mode: String::new(),
        linked_files: 0,
        copied_files: 0,
        downloaded_files: 0,
        store_files: 0,
        reused_bytes: 0,
        store_bytes: 0,
        downloaded_bytes: 0,
        resumed_files: 0,
        resumed_bytes: 0,
        pruned_files: 0,
        files: Vec::new(),
        verified: false,
    };

    let store = opts.store_root.map(SharedStore::new);

    // Candidate roots, Workshop first so its provenance is preferred.
    let mut roots: Vec<(PathBuf, Origin)> = opts
        .workshop_roots
        .iter()
        .cloned()
        .map(|root| (root, Origin::Workshop))
        .collect();
    if let Some(store) = &store {
        let dir = store.mod_dir(&manifest.name);
        if dir.is_dir() {
            roots.push((dir, Origin::Store));
        }
    }

    let complete_root = find_complete_root(manifest, &roots, opts.cache);
    let effective = match opts.mode {
        LinkMode::Passthrough | LinkMode::DirLink if complete_root.is_none() => LinkMode::Overlay,
        LinkMode::Auto => {
            if complete_root.is_some() {
                LinkMode::DirLink
            } else {
                LinkMode::Overlay
            }
        }
        other => other,
    };

    let mut stats = LockStats {
        logical_bytes: manifest.total_size(),
        ..Default::default()
    };

    if effective == LinkMode::Passthrough {
        // Passthrough launches from the source; a leftover folder would only
        // confuse Arma.
        if linker::exists(target) {
            linker::remove_path(target)?;
        }
        let (root, origin) = complete_root.expect("checked above");
        report.mode = "passthrough".into();
        report.linked_files = manifest.files.len();
        match origin {
            Origin::Workshop => {
                report.reused_bytes = manifest.total_size();
                stats.workshop_bytes = manifest.total_size();
            }
            Origin::Store => {
                report.store_bytes = manifest.total_size();
                stats.store_bytes = manifest.total_size();
            }
        }
        report.verified = true;
        report.target = root.display().to_string();
        report.files = whole_mod_files(manifest, origin, &root, "passthrough");
        emit(
            opts,
            ProgressEvent::ModLinked {
                mod_name: &manifest.name,
                action: match origin {
                    Origin::Workshop => FileAction::Workshop,
                    Origin::Store => FileAction::Store,
                },
                bytes: manifest.total_size(),
            },
        );
        write_lock_full(
            target,
            manifest,
            opts.repo,
            "passthrough",
            BTreeMap::new(),
            stats,
        )?;
        return Ok(report);
    }

    if effective == LinkMode::DirLink {
        let (root, origin) = complete_root.expect("checked above");
        if linker::exists(target) {
            linker::remove_path(target)?;
        }
        linker::link_dir(target, &root)?;
        report.linked_files = manifest.files.len();
        report.mode = "dir-link".into();
        report.files = whole_mod_files(
            manifest,
            origin,
            &root,
            if linker::is_link(target) {
                "junction"
            } else {
                "copy"
            },
        );
        match origin {
            Origin::Workshop => {
                report.reused_bytes = manifest.total_size();
                stats.workshop_bytes = manifest.total_size();
            }
            Origin::Store => {
                report.store_bytes = manifest.total_size();
                stats.store_bytes = manifest.total_size();
            }
        }
        emit(
            opts,
            ProgressEvent::ModLinked {
                mod_name: &manifest.name,
                action: match origin {
                    Origin::Workshop => FileAction::Workshop,
                    Origin::Store => FileAction::Store,
                },
                bytes: manifest.total_size(),
            },
        );
        write_lock_full(
            target,
            manifest,
            opts.repo,
            "dir-link",
            BTreeMap::new(),
            stats,
        )?;
    } else {
        fs::create_dir_all(target)?;
        // Claim the folder before touching it: if this run is cancelled or
        // crashes, the next one recognises it as resumable instead of refusing
        // to replace it.
        write_lock_full(
            target,
            manifest,
            opts.repo,
            "partial",
            BTreeMap::new(),
            stats,
        )?;
        let mut sources = BTreeMap::new();

        // Phase 1: everything the Workshop cache, the store or the previous run
        // already has is linked or kept immediately; those are cheap.
        let mut fetches: Vec<&ModFile> = Vec::new();
        for file in &manifest.files {
            check_cancel(opts)?;
            emit(
                opts,
                ProgressEvent::FileStart {
                    mod_name: &manifest.name,
                    path: &file.path,
                    bytes: file.length,
                },
            );
            let dest = join_rel(target, &file.path)?;

            // Resume: keep a file the previous run already placed correctly.
            if resume && check_file_shared(file, &dest, opts.cache) == FileStatus::Ok {
                let origin = previous
                    .as_ref()
                    .and_then(|lock| lock.files.get(&file.path).cloned())
                    .unwrap_or_else(|| format!("reuse:{}", dest.display()));
                account_kept(&origin, file.length, &mut report, &mut stats);
                sources.insert(file.path.clone(), origin.clone());
                let file_origin = crate::tracking::FileOrigin::from_lock_entry(&origin)
                    .unwrap_or(crate::tracking::FileOrigin::Reuse);
                report.files.push(InstalledFile {
                    path: file.path.clone(),
                    bytes: file.length,
                    checksum: file.checksum.clone(),
                    origin: file_origin,
                    location: location_for(&origin, store.is_some()),
                    link: "existing".to_string(),
                    source: origin.split_once(':').map(|(_, path)| path.to_string()),
                });
                emit(
                    opts,
                    ProgressEvent::File {
                        mod_name: &manifest.name,
                        path: &file.path,
                        action: FileAction::Reuse,
                        bytes: file.length,
                    },
                );
                continue;
            }

            let Some((src, origin)) = matches_for_file(file, &roots, opts.cache) else {
                fetches.push(file);
                continue;
            };

            if linker::exists(&dest) {
                linker::remove_path(&dest)?;
            }
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)?;
            }
            let kind = place(&dest, &src, effective == LinkMode::Copy)?;
            let action = match (origin, kind) {
                (_, LinkKind::Copy) => FileAction::Copy,
                (Origin::Workshop, _) => FileAction::Workshop,
                (Origin::Store, _) => FileAction::Store,
            };
            match kind {
                LinkKind::Copy => report.copied_files += 1,
                _ => report.linked_files += 1,
            }
            match origin {
                Origin::Workshop => {
                    report.reused_bytes += file.length;
                    stats.workshop_bytes += file.length;
                }
                Origin::Store => {
                    report.store_files += 1;
                    report.store_bytes += file.length;
                    stats.store_bytes += file.length;
                }
            }
            emit(
                opts,
                ProgressEvent::File {
                    mod_name: &manifest.name,
                    path: &file.path,
                    action,
                    bytes: file.length,
                },
            );
            let label = match origin {
                Origin::Workshop => "workshop",
                Origin::Store => "store",
            };
            let entry = format!("{label}:{}", src.display());
            sources.insert(file.path.clone(), entry);
            report.files.push(InstalledFile {
                path: file.path.clone(),
                bytes: file.length,
                checksum: file.checksum.clone(),
                origin: match origin {
                    Origin::Workshop => crate::tracking::FileOrigin::Workshop,
                    Origin::Store => crate::tracking::FileOrigin::Store,
                },
                location: match origin {
                    Origin::Workshop => crate::tracking::FileLocation::Workshop,
                    Origin::Store => crate::tracking::FileLocation::Store,
                },
                link: link_label(kind).to_string(),
                source: Some(src.display().to_string()),
            });
        }

        if report.resumed_files > 0 {
            emit(
                opts,
                ProgressEvent::Resumed {
                    mod_name: &manifest.name,
                    files: report.resumed_files,
                    bytes: report.resumed_bytes,
                },
            );
        }

        // Phase 2: fetch the rest. Files are downloaded in waves; the worker
        // count adapts to whether the disk or the network is the bottleneck.
        let payload_for = |file: &ModFile| -> Result<PathBuf> {
            match &store {
                Some(store) => store.file_path(&manifest.name, &file.path),
                None => join_rel(target, &file.path),
            }
        };

        let max_parallel = opts.max_parallel.clamp(1, 32);
        // Start conservatively so the limiter can climb when the link is the
        // bottleneck, and drop when the disk is.
        let mut limit = max_parallel.clamp(1, 4);
        let mut streak = 0i32;
        let mut queue = fetches;

        while !queue.is_empty() {
            check_cancel(opts)?;
            let take = limit.min(queue.len());
            let batch: Vec<&ModFile> = queue.drain(..take).collect();

            let results: Vec<Result<Fetched>> = std::thread::scope(|scope| {
                let handles: Vec<_> = batch
                    .iter()
                    .map(|file| {
                        scope.spawn(move || {
                            let payload = payload_for(file)?;
                            // A payload left over from an interrupted run is
                            // already complete: do not download it again.
                            if check_file_shared(file, &payload, opts.cache) == FileStatus::Ok {
                                return Ok(Fetched {
                                    bytes: 0,
                                    network: false,
                                    existing: true,
                                    ..Default::default()
                                });
                            }
                            let on_bytes = |done: u64| {
                                emit(
                                    opts,
                                    ProgressEvent::FileBytes {
                                        mod_name: &manifest.name,
                                        path: &file.path,
                                        done,
                                        total: file.length,
                                    },
                                );
                            };
                            fetch_payload(manifest, file, &payload, opts, &on_bytes)
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|handle| match handle.join() {
                        Ok(result) => result,
                        Err(_) => Err(Error::msg("download worker panicked")),
                    })
                    .collect()
            });

            // Phase 3: place the freshly fetched payloads and record them.
            let mut wave_elapsed = 0u64;
            let mut wave_write = 0u64;
            for (file, fetched) in batch.iter().zip(results) {
                let fetched = fetched?;
                wave_elapsed += fetched.elapsed_nanos;
                wave_write += fetched.write_nanos;

                let dest = join_rel(target, &file.path)?;
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent)?;
                }
                let (action, kind) = if let Some(store) = &store {
                    let payload = store.file_path(&manifest.name, &file.path)?;
                    let kind = place(&dest, &payload, effective == LinkMode::Copy)?;
                    let action = if fetched.existing {
                        FileAction::Reuse
                    } else {
                        match (fetched.network, kind) {
                            (true, LinkKind::Copy) => FileAction::Copy,
                            (true, _) => FileAction::Download,
                            // A local mirror never touches the network.
                            (false, _) => FileAction::Copy,
                        }
                    };
                    sources.insert(file.path.clone(), format!("store:{}", payload.display()));
                    (action, kind)
                } else {
                    let rel = format!("{}/{}", manifest.name, file.path);
                    sources.insert(file.path.clone(), format!("download:{rel}"));
                    let action = if fetched.existing {
                        FileAction::Reuse
                    } else if fetched.network {
                        FileAction::Download
                    } else {
                        FileAction::Copy
                    };
                    (action, LinkKind::Existing)
                };

                if fetched.existing {
                    report.resumed_files += 1;
                    report.resumed_bytes += file.length;
                }
                match kind {
                    LinkKind::Copy => report.copied_files += 1,
                    LinkKind::Existing => {
                        if !fetched.network && !fetched.existing {
                            report.copied_files += 1;
                        }
                    }
                    _ => report.linked_files += 1,
                }
                if fetched.network {
                    report.downloaded_files += 1;
                    report.downloaded_bytes += fetched.bytes;
                    stats.downloaded_bytes += fetched.bytes;
                }
                if store.is_some() {
                    report.store_bytes += file.length;
                    if effective == LinkMode::Copy {
                        stats.copied_bytes += file.length;
                    } else {
                        stats.store_bytes += file.length;
                    }
                } else if effective == LinkMode::Copy {
                    stats.copied_bytes += file.length;
                }
                emit(
                    opts,
                    ProgressEvent::File {
                        mod_name: &manifest.name,
                        path: &file.path,
                        action,
                        bytes: file.length,
                    },
                );

                let (file_origin, location) =
                    match (store.is_some(), fetched.network, fetched.existing) {
                        // Already complete in the store from an earlier run.
                        (true, _, true) => (
                            crate::tracking::FileOrigin::Store,
                            crate::tracking::FileLocation::Store,
                        ),
                        // Fetched into the shared store.
                        (true, true, false) => (
                            crate::tracking::FileOrigin::Download,
                            crate::tracking::FileLocation::Store,
                        ),
                        // A local mirror filled the store.
                        (true, false, false) => (
                            crate::tracking::FileOrigin::Copy,
                            crate::tracking::FileLocation::Store,
                        ),
                        // No store: the bytes live in the workspace.
                        (false, true, _) => (
                            crate::tracking::FileOrigin::Download,
                            crate::tracking::FileLocation::Workspace,
                        ),
                        (false, false, _) => (
                            crate::tracking::FileOrigin::Copy,
                            crate::tracking::FileLocation::Workspace,
                        ),
                    };
                report.files.push(InstalledFile {
                    path: file.path.clone(),
                    bytes: file.length,
                    checksum: file.checksum.clone(),
                    origin: file_origin,
                    location,
                    link: link_label(kind).to_string(),
                    source: sources.get(&file.path).cloned(),
                });
            }

            if limit > 1 && wave_elapsed > 0 {
                let share = wave_write as f64 / wave_elapsed as f64;
                let next = adjust_parallel(limit, max_parallel, share, &mut streak);
                if next != limit {
                    let message = if next < limit {
                        format!("reducing parallel downloads to {next} (disk is busy)")
                    } else {
                        format!("raising parallel downloads to {next}")
                    };
                    emit(opts, ProgressEvent::Note { message: &message });
                    limit = next;
                }
            }
        }

        report.mode = if effective == LinkMode::Copy {
            "copy".into()
        } else {
            "overlay".into()
        };
        // A reused folder may still hold files the manifest no longer lists.
        if resume {
            report.pruned_files = prune_strays(target, manifest)?;
        }
        manifest.write_to(target, SrfFormat::V3)?;
        write_lock_full(target, manifest, opts.repo, &report.mode, sources, stats)?;
    }

    let verify = verify_dir_cached(manifest, target, opts.cache)?;
    report.verified = verify.files_ok();
    if !report.verified {
        let bad: Vec<String> = verify
            .mismatches()
            .map(|f| format!("{}: {:?}", f.path, f.status))
            .collect();
        return Err(Error::msg(format!(
            "{} failed verification after install: {}",
            manifest.name,
            bad.join(", ")
        )));
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::generate_from_dir;

    const FILES: &[(&str, &[u8])] = &[
        ("mod.cpp", b"name=\"demo\";\n"),
        ("addons/a.pbo", b"payload-bytes"),
    ];

    fn repo_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let mod_dir = dir.path().join("@demo");
        fs::create_dir_all(&mod_dir).unwrap();
        for (rel, data) in FILES {
            let path = mod_dir.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, data).unwrap();
        }
        dir
    }

    fn manifest_from(dir: &Path) -> ModManifest {
        generate_from_dir("@demo", &dir.join("@demo")).unwrap()
    }

    #[test]
    fn store_is_filled_once_and_linked_into_workspaces() {
        let repo = repo_dir();
        let manifest = manifest_from(repo.path());
        let store_dir = tempfile::tempdir().unwrap();
        let ws1 = tempfile::tempdir().unwrap();
        let ws2 = tempfile::tempdir().unwrap();

        let first = install_mod(
            &manifest,
            &ws1.path().join("@demo"),
            &InstallOptions {
                store_root: Some(store_dir.path()),
                local_repo: Some(repo.path()),
                repo: Some("https://example/"),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(first.mode, "overlay");
        assert_eq!(first.downloaded_files, 0, "local mirror is not a download");
        assert_eq!(first.linked_files, manifest.files.len());
        assert_eq!(first.store_bytes, manifest.total_size());

        // Second workspace: everything must come from the store, no downloads.
        let second = install_mod(
            &manifest,
            &ws2.path().join("@demo"),
            &InstallOptions {
                store_root: Some(store_dir.path()),
                local_repo: Some(repo.path()),
                repo: Some("https://example/"),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(second.downloaded_files, 0, "store must satisfy every file");
        assert_eq!(second.downloaded_bytes, 0);
        // A complete store behaves like a complete Workshop item: dir-link it.
        assert_eq!(second.mode, "dir-link");
        assert_eq!(second.store_bytes, manifest.total_size());
        assert_eq!(second.reused_bytes, 0);

        let lock = crate::workspace::read_lock(&ws2.path().join("@demo")).unwrap();
        assert_eq!(lock.repo.as_deref(), Some("https://example/"));
        assert_eq!(lock.stats.logical_bytes, manifest.total_size());
        assert_eq!(lock.stats.store_bytes, manifest.total_size());
        assert_eq!(lock.stats.workshop_bytes, 0);

        // A third install of the same mod elsewhere is also a link, no traffic.
        let third = install_mod(
            &manifest,
            &ws2.path().join("@demo2"),
            &InstallOptions {
                store_root: Some(store_dir.path()),
                repo: Some("https://example/"),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(third.mode, "dir-link");
        assert_eq!(third.store_bytes, manifest.total_size());
    }

    #[test]
    fn progress_events_cover_every_byte_exactly_once() {
        let repo = repo_dir();
        let manifest = manifest_from(repo.path());
        let ws = tempfile::tempdir().unwrap();
        let store_dir = tempfile::tempdir().unwrap();

        use std::sync::Mutex;
        let seen = Mutex::new(Vec::<(String, u64, FileAction)>::new());
        let callback = |event: ProgressEvent<'_>| {
            let mut seen = seen.lock().unwrap();
            match event {
                ProgressEvent::File {
                    path,
                    action,
                    bytes,
                    ..
                } => seen.push((path.to_string(), bytes, action)),
                ProgressEvent::ModLinked {
                    mod_name,
                    action,
                    bytes,
                } => seen.push((format!("{mod_name}/*"), bytes, action)),
                ProgressEvent::FileStart { .. } | ProgressEvent::FileBytes { .. } => {}
                ProgressEvent::Note { .. }
                | ProgressEvent::Retry { .. }
                | ProgressEvent::Resumed { .. } => {}
            }
        };

        let report = install_mod(
            &manifest,
            &ws.path().join("@demo"),
            &InstallOptions {
                store_root: Some(store_dir.path()),
                local_repo: Some(repo.path()),
                on_progress: Some(&callback),
                ..Default::default()
            },
        )
        .unwrap();

        let total: u64 = seen.lock().unwrap().iter().map(|(_, bytes, _)| bytes).sum();
        assert_eq!(total, manifest.total_size());
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), manifest.files.len());
        assert!(seen.iter().all(|(_, bytes, _)| *bytes > 0));
        // A local mirror is a copy, not a download.
        assert!(seen
            .iter()
            .all(|(_, _, action)| *action == FileAction::Copy));
        drop(seen);
        // Placement is a link into the workspace even though the bytes were
        // copied out of the local mirror into the store.
        assert_eq!(report.linked_files, manifest.files.len());
    }

    #[test]
    fn dir_link_reports_one_event_for_the_whole_mod() {
        let repo = repo_dir();
        let manifest = manifest_from(repo.path());
        let ws = tempfile::tempdir().unwrap();

        use std::sync::Mutex;
        let events = Mutex::new(Vec::<(FileAction, u64)>::new());
        let callback = |event: ProgressEvent<'_>| match event {
            ProgressEvent::ModLinked { action, bytes, .. } => {
                events.lock().unwrap().push((action, bytes))
            }
            ProgressEvent::Note { .. }
            | ProgressEvent::Retry { .. }
            | ProgressEvent::Resumed { .. } => {}
            ProgressEvent::FileStart { .. } | ProgressEvent::FileBytes { .. } => {}
            ProgressEvent::File { .. } => panic!("dir-link must not emit per-file events"),
        };

        install_mod(
            &manifest,
            &ws.path().join("@demo"),
            &InstallOptions {
                workshop_roots: &[repo.path().join("@demo")],
                on_progress: Some(&callback),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(
            *events.lock().unwrap(),
            vec![(FileAction::Workshop, manifest.total_size())]
        );
    }

    #[test]
    fn parallel_limit_adapts_to_disk_pressure() {
        // Two high-write-share samples in a row lower the limit.
        let mut streak = 0;
        let mut limit = 8;
        limit = adjust_parallel(limit, 8, 0.9, &mut streak);
        assert_eq!(limit, 8, "one sample is not enough");
        limit = adjust_parallel(limit, 8, 0.9, &mut streak);
        assert_eq!(limit, 7);

        // Network-bound samples raise it again.
        let mut streak = 0;
        let mut limit = 2;
        limit = adjust_parallel(limit, 8, 0.05, &mut streak);
        limit = adjust_parallel(limit, 8, 0.05, &mut streak);
        assert_eq!(limit, 3);

        // Never below 1 or above max.
        let mut streak = 0;
        let mut limit = 1;
        for _ in 0..4 {
            limit = adjust_parallel(limit, 8, 0.95, &mut streak);
        }
        assert_eq!(limit, 1);

        let mut streak = 0;
        let mut limit = 8;
        for _ in 0..4 {
            limit = adjust_parallel(limit, 8, 0.0, &mut streak);
        }
        assert_eq!(limit, 8);

        // A balanced share keeps it steady.
        let mut streak = 0;
        assert_eq!(adjust_parallel(5, 8, 0.35, &mut streak), 5);
    }

    #[test]
    fn parallel_downloads_still_verify_and_account_every_byte() {
        let repo = repo_dir();
        let manifest = manifest_from(repo.path());
        let ws = tempfile::tempdir().unwrap();
        let store_dir = tempfile::tempdir().unwrap();

        let report = install_mod(
            &manifest,
            &ws.path().join("@demo"),
            &InstallOptions {
                store_root: Some(store_dir.path()),
                local_repo: Some(repo.path()),
                max_parallel: 4,
                ..Default::default()
            },
        )
        .unwrap();

        assert!(report.verified);
        assert_eq!(report.store_bytes, manifest.total_size());
        let lock = crate::workspace::read_lock(&ws.path().join("@demo")).unwrap();
        assert_eq!(lock.stats.logical_bytes, manifest.total_size());
    }

    #[test]
    fn chunks_merge_contiguous_parts() {
        use crate::formats::FilePart;
        let part = |start: u64, length: u64| FilePart {
            path: "p".into(),
            start,
            length,
            checksum: "0".into(),
        };
        let parts = vec![part(0, 100), part(100, 100), part(200, 100), part(300, 100)];

        // Everything fits in one chunk.
        let chunks = plan_chunks(&parts, 1024);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].start, 0);
        assert_eq!(chunks[0].len, 400);
        assert_eq!((chunks[0].first_part, chunks[0].last_part), (0, 3));

        // 250 bytes per chunk: 0-199 and 200-399.
        let chunks = plan_chunks(&parts, 250);
        assert_eq!(chunks.len(), 2);
        assert_eq!((chunks[0].start, chunks[0].len), (0, 200));
        assert_eq!((chunks[1].start, chunks[1].len), (200, 200));
        assert_eq!((chunks[1].first_part, chunks[1].last_part), (2, 3));

        // A part bigger than the chunk budget becomes its own chunk.
        let big = vec![part(0, 4096), part(4096, 10)];
        let chunks = plan_chunks(&big, 1024);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len, 4096);
        assert_eq!(chunks[1].len, 10);
    }

    #[test]
    fn chunks_cover_every_part_exactly_once() {
        use crate::formats::FilePart;
        let mut parts = Vec::new();
        let mut start = 0u64;
        for index in 0..97u64 {
            let length = 1 + (index * 7919) % 500_000;
            parts.push(FilePart {
                path: format!("p{index}"),
                start,
                length,
                checksum: "0".into(),
            });
            start += length;
        }
        for budget in [4096u64, 1 << 20, 8 << 20] {
            let chunks = plan_chunks(&parts, budget);
            let mut covered = 0usize;
            let mut offset = 0u64;
            for chunk in &chunks {
                assert_eq!(chunk.start, offset, "chunks must be contiguous");
                covered += chunk.last_part - chunk.first_part + 1;
                offset += chunk.len;
            }
            assert_eq!(covered, parts.len());
            assert_eq!(offset, start);
        }
    }

    #[test]
    fn failed_install_leaves_no_partial_folder() {
        let repo = repo_dir();
        let manifest = manifest_from(repo.path());
        let ws = tempfile::tempdir().unwrap();
        let target = ws.path().join("@demo");

        // No sources configured at all: the install must fail cleanly.
        let err = install_mod(&manifest, &target, &InstallOptions::default()).unwrap_err();
        assert!(err.to_string().contains("missing file"), "got: {err}");
        assert!(
            !linker::exists(&target),
            "a failed install must not leave a folder that blocks the retry"
        );

        // The retry works without --force.
        let report = install_mod(
            &manifest,
            &target,
            &InstallOptions {
                local_repo: Some(repo.path()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(report.verified);
    }

    #[test]
    fn cancel_keeps_verified_files_and_the_retry_resumes() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let repo = repo_dir();
        let manifest = manifest_from(repo.path());
        let ws = tempfile::tempdir().unwrap();
        let target = ws.path().join("@demo");

        // Stop right after the first file lands, like a user pressing Cancel.
        let cancel = AtomicBool::new(false);
        let callback = |event: ProgressEvent<'_>| {
            if matches!(event, ProgressEvent::File { .. }) {
                cancel.store(true, Ordering::Relaxed);
            }
        };
        let err = install_mod(
            &manifest,
            &target,
            &InstallOptions {
                local_repo: Some(repo.path()),
                cancel: Some(&cancel),
                on_progress: Some(&callback),
                // One file per wave, so the cancel lands between two files.
                max_parallel: 1,
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.is_cancelled(), "expected a cancel, got: {err}");

        let placed = manifest
            .files
            .iter()
            .filter(|file| target.join(&file.path).is_file())
            .count();
        assert_eq!(placed, 1, "the cancelled run stops after one file");
        assert!(
            crate::workspace::is_managed(&target),
            "a partial folder must stay managed so the retry may reuse it"
        );

        // Retry: the finished file is verified and kept, the rest is installed.
        let report = install_mod(
            &manifest,
            &target,
            &InstallOptions {
                local_repo: Some(repo.path()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(report.resumed_files, 1);
        assert_eq!(report.resumed_bytes, manifest.files[0].length);
        assert!(report.verified);
        assert_eq!(
            report.copied_files + report.linked_files,
            manifest.files.len() - 1,
            "only the missing files are touched"
        );
    }

    #[test]
    fn resume_prunes_files_that_left_the_manifest() {
        let repo = repo_dir();
        let manifest = manifest_from(repo.path());
        let ws = tempfile::tempdir().unwrap();
        let target = ws.path().join("@demo");
        let opts = InstallOptions {
            local_repo: Some(repo.path()),
            ..Default::default()
        };
        install_mod(&manifest, &target, &opts).unwrap();

        fs::write(target.join("stale.pbo"), b"gone from the repo").unwrap();
        let report = install_mod(&manifest, &target, &opts).unwrap();
        assert_eq!(report.pruned_files, 1);
        assert!(!target.join("stale.pbo").exists());
        assert!(report.verified);
    }

    #[test]
    fn no_store_downloads_directly_into_workspace() {
        let repo = repo_dir();
        let manifest = manifest_from(repo.path());
        let ws = tempfile::tempdir().unwrap();

        let report = install_mod(
            &manifest,
            &ws.path().join("@demo"),
            &InstallOptions {
                local_repo: Some(repo.path()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(report.downloaded_files, 0, "local mirror is not a download");
        assert_eq!(report.copied_files, manifest.files.len());
        assert_eq!(report.store_bytes, 0);
        assert!(ws.path().join("@demo/mod.cpp").is_file());
    }
}
