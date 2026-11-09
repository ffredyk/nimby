//! Nimby core: Swifty-compatible repository formats, verification, Workshop
//! reuse and install engine.

pub mod checksum;
pub mod config;
pub mod error;
pub mod formats;
pub mod graphics;
pub mod hashcache;
pub mod http;
pub mod install;
pub mod launch;
pub mod linker;
pub mod params;
pub mod pbo;
pub mod retry;
pub mod stats;
pub mod steam;
pub mod store;
pub mod tracking;
pub mod util;
pub mod verify;
pub mod workspace;

pub use config::{AppConfig, RepoEntry};
pub use error::{Error, Result};
pub use formats::{FileKind, FilePart, ModFile, ModManifest, RepoManifest, RepoMod, SrfFormat};
pub use install::{
    install_mod, FileAction, InstallOptions, InstallReport, InstalledFile, ProgressEvent,
    ProgressFn,
};
pub use linker::{LinkKind, LinkMode};
pub use params::{LaunchParam, ParamKind, ParamSpec};
pub use retry::{with_retry, RetryPolicy};
pub use stats::{collect as collect_stats, RepoStats, StatsReport};
pub use store::SharedStore;
pub use tracking::{
    FileLocation, FileOrigin, FileTrack, ModTrack, RepoIndex, TrackCheck, TrackTotals, UpdateState,
};
pub use verify::{verify_dir, FileStatus, VerifyReport};
pub use workspace::{
    installed_manifest, installed_mode, is_managed, lock_path, read_lock, write_lock, LockStats,
};
