use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};

mod app;
mod control;
mod publish;
mod settings;

#[derive(Parser)]
#[command(
    name = "nimby",
    version,
    about = "Swifty-compatible Arma 3 addon manager with Workshop and shared-store reuse"
)]
pub struct Cli {
    /// Machine readable output.
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum FormatArg {
    V3,
    Legacy,
}

impl From<FormatArg> for nimby_core::SrfFormat {
    fn from(value: FormatArg) -> Self {
        match value {
            FormatArg::V3 => nimby_core::SrfFormat::V3,
            FormatArg::Legacy => nimby_core::SrfFormat::Legacy,
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum ProgressArg {
    /// One JSON object per line, for the GUI.
    Json,
    /// Human readable progress on stdout.
    Text,
}

#[derive(Subcommand)]
pub enum Command {
    /// Generate a mod.srf for a mod folder.
    Srf {
        dir: PathBuf,
        #[arg(long, value_enum, default_value = "v3")]
        format: FormatArg,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Verify a mod folder against its manifest.
    Verify {
        dir: PathBuf,
        #[arg(long)]
        srf: Option<PathBuf>,
        /// Also fail when files exist that the manifest does not list.
        #[arg(long)]
        strict: bool,
    },
    /// Build a Swifty repository from a source folder of @mods.
    Publish {
        #[arg(long)]
        src: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        name: Option<String>,
        #[arg(long, value_enum, default_value = "v3")]
        format: FormatArg,
        /// Comma separated list of mod folders to publish.
        #[arg(long)]
        mods: Option<String>,
    },
    /// Show a remote repository summary, optionally caching its artwork.
    Inspect {
        repo: String,
        #[arg(long)]
        mods: Option<String>,
        /// Download and cache repo.png / icon.png.
        #[arg(long)]
        images: bool,
    },
    /// Install mods from a repository, reusing Workshop and shared-store content.
    Install {
        /// Repository URL. Defaults to the repo selected in the config.
        repo: Option<String>,
        /// Workspace folder. Defaults to the config value.
        #[arg(long)]
        path: Option<PathBuf>,
        #[arg(long)]
        mods: Option<String>,
        #[arg(long)]
        optional: Option<String>,
        /// Install only the mods named by `--mods`/`--optional`, never the
        /// repository's required set.
        #[arg(long)]
        no_required: bool,
        #[arg(long)]
        mode: Option<String>,
        #[arg(long)]
        no_workshop: bool,
        /// Ignore the shared mod store for this run.
        #[arg(long)]
        no_store: bool,
        #[arg(long)]
        force: bool,
        /// Stream progress: `json` emits one JSON object per line.
        #[arg(long, value_enum, conflicts_with = "json")]
        progress: Option<ProgressArg>,
        /// Maximum concurrent downloads (defaults to the configured value).
        #[arg(long)]
        parallel: Option<u32>,
        /// How many mods are installed at the same time (defaults to the
        /// configured value).
        #[arg(long)]
        mod_parallel: Option<u32>,
        /// What to do when a mod cannot be installed after the automatic
        /// retries: abort, retry, skip, or ask over the control channel.
        #[arg(long, value_enum, default_value = "abort")]
        on_error: control::OnError,
        /// Read control words (cancel, retry, skip, abort) from stdin.
        #[arg(long)]
        control_stdin: bool,
        /// Extra attempts per mod before the error is reported.
        #[arg(long, default_value_t = 3)]
        max_retries: u32,
    },
    /// Show what is installed in a workspace.
    Status {
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Disk usage per repository and how much space linking saved.
    Stats {
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Compare installed mods with their repositories.
    Update {
        /// Only report what would change.
        #[arg(long)]
        check: bool,
        /// Hash every file instead of trusting the tracking index.
        #[arg(long)]
        deep: bool,
    },
    /// Show the per-file tracking index of an install folder.
    Track {
        #[arg(long)]
        path: Option<PathBuf>,
        /// Show one mod only.
        #[arg(long)]
        mod_name: Option<String>,
        /// Re-hash the files instead of trusting size and modification time.
        #[arg(long)]
        verify: bool,
        /// Backfill the index from the lock files (no network, no hashing).
        #[arg(long)]
        rebuild: bool,
    },
    /// Reinstall mods that no longer match their manifest.
    Repair,
    /// Migrate settings from an existing Swifty installation.
    Migrate {
        /// Settings.json to read (defaults to the Swifty client's).
        #[arg(long)]
        source: Option<PathBuf>,
        #[arg(long)]
        dry_run: bool,
        /// Instead of Swifty settings, move mods that sit directly in the
        /// workspace root into the active repository's subfolder.
        #[arg(long)]
        layout: bool,
    },
    /// Shared mod store maintenance.
    Store {
        #[command(subcommand)]
        action: StoreAction,
    },
    /// Environment checks: Steam, Arma, Workshop, link capabilities.
    Doctor,
    /// Launch Arma 3 with every installed mod.
    Launch {
        #[arg(long)]
        arma: Option<PathBuf>,
        #[arg(long)]
        path: Option<PathBuf>,
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        server: Option<String>,
        #[arg(long)]
        battleye: bool,
        /// Extra parameters for this run only, repeatable.
        #[arg(long = "param", allow_hyphen_values = true)]
        params: Vec<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Read or change the persisted configuration.
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,
    },
    /// Manage the list of known repositories.
    Repo {
        #[command(subcommand)]
        action: RepoAction,
    },
}

#[derive(Subcommand)]
pub enum StoreAction {
    /// Show how much the store holds.
    Info,
    /// Delete store folders that contain no files.
    Prune,
}

#[derive(Subcommand)]
pub enum ConfigAction {
    /// Update one or more settings. Omitted settings stay unchanged.
    Set {
        #[arg(long)]
        arma: Option<PathBuf>,
        /// Reset the Arma path so it is autodetected again.
        #[arg(long)]
        clear_arma: bool,
        #[arg(long)]
        temp: Option<PathBuf>,
        #[arg(long)]
        clear_temp: bool,
        #[arg(long)]
        shared: Option<PathBuf>,
        /// Disable the shared store.
        #[arg(long)]
        clear_shared: bool,
        /// Default install folder, used by repositories without their own path.
        #[arg(long)]
        workspace: Option<PathBuf>,
        #[arg(long)]
        link_mode: Option<String>,
        /// Maximum concurrent file downloads per mod.
        #[arg(long)]
        parallel: Option<u32>,
        /// How many mods are installed at the same time.
        #[arg(long)]
        mod_parallel: Option<u32>,
        #[arg(long)]
        battleye: Option<bool>,
        /// Remove every configured launch parameter first.
        #[arg(long)]
        clear_params: bool,
        /// Launch parameter, for example `-name=Fred` or `-noSplash`.
        #[arg(long = "param", allow_hyphen_values = true)]
        params: Vec<String>,
    },
}

#[derive(Subcommand)]
pub enum RepoAction {
    List,
    Add {
        url: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        user: Option<String>,
        #[arg(long)]
        pass: Option<String>,
        /// Subfolder of the workspace root this repo installs into.
        #[arg(long)]
        folder: Option<String>,
        /// Make it the active repo.
        #[arg(long)]
        select: bool,
    },
    /// Show or change the subfolder a repository installs into.
    Folder {
        url: String,
        /// New subfolder name. Omit to print the current one; use `--default`
        /// to derive it from the repo name again.
        folder: Option<String>,
        #[arg(long)]
        default: bool,
    },
    Remove {
        url: String,
    },
    Select {
        url: String,
    },
}

fn main() {
    let cli = Cli::parse();
    if let Err(err) = run(cli) {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    let json = cli.json;
    match cli.command {
        Command::Srf { dir, format, out } => {
            publish::cmd_srf(&dir, format.into(), out.as_deref(), json)
        }
        Command::Verify { dir, srf, strict } => {
            publish::cmd_verify(&dir, srf.as_deref(), strict, json)
        }
        Command::Publish {
            src,
            out,
            config,
            name,
            format,
            mods,
        } => publish::cmd_publish(
            &src,
            &out,
            config.as_deref(),
            name.as_deref(),
            format.into(),
            mods.as_deref(),
            json,
        ),
        Command::Inspect { repo, mods, images } => {
            publish::cmd_inspect(&repo, mods.as_deref(), images, json)
        }
        Command::Install {
            repo,
            path,
            mods,
            optional,
            no_required,
            mode,
            no_workshop,
            no_store,
            force,
            progress,
            parallel,
            mod_parallel,
            on_error,
            control_stdin,
            max_retries,
        } => app::cmd_install(
            repo.as_deref(),
            path.as_deref(),
            mods.as_deref(),
            optional.as_deref(),
            no_required,
            mode.as_deref(),
            no_workshop,
            no_store,
            force,
            progress,
            parallel,
            mod_parallel,
            on_error,
            if control_stdin {
                control::Control::stdin()
            } else {
                control::Control::none()
            },
            max_retries,
            json,
        ),
        Command::Status { path } => app::cmd_status(path.as_deref(), json),
        Command::Stats { path } => app::cmd_stats(path.as_deref(), json),
        Command::Update { check, deep } => app::cmd_update(check, deep, json),
        Command::Track {
            path,
            mod_name,
            verify,
            rebuild,
        } => app::cmd_track(path.as_deref(), mod_name.as_deref(), verify, rebuild, json),
        Command::Repair => app::cmd_repair(json),
        Command::Migrate {
            source,
            dry_run,
            layout,
        } => {
            if layout {
                app::cmd_migrate_layout(dry_run, json)
            } else {
                app::cmd_migrate(source.as_deref(), dry_run, json)
            }
        }
        Command::Store { action } => match action {
            StoreAction::Info => app::cmd_store_info(json),
            StoreAction::Prune => app::cmd_store_prune(json),
        },
        Command::Doctor => app::cmd_doctor(json),
        Command::Launch {
            arma,
            path,
            repo,
            server,
            battleye,
            params,
            dry_run,
        } => app::cmd_launch(
            arma.as_deref(),
            path.as_deref(),
            repo.as_deref(),
            server.as_deref(),
            battleye,
            &params,
            dry_run,
            json,
        ),
        Command::Config { action } => match action {
            None => settings::cmd_config_show(json),
            Some(ConfigAction::Set {
                arma,
                clear_arma,
                temp,
                clear_temp,
                shared,
                clear_shared,
                workspace,
                link_mode,
                parallel,
                mod_parallel,
                battleye,
                clear_params,
                params,
            }) => settings::cmd_config_set(settings::SetArgs {
                arma,
                clear_arma,
                temp,
                clear_temp,
                shared,
                clear_shared,
                workspace,
                link_mode,
                parallel,
                mod_parallel,
                battleye,
                clear_params,
                params,
                json,
            }),
        },
        Command::Repo { action } => settings::cmd_repo(action, json),
    }
}

/// Split a comma separated mod list.
pub fn split_list(value: Option<&str>) -> Option<Vec<String>> {
    value.map(|v| {
        v.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    })
}
