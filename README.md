# Nimby

A modern Arma 3 addon manager that stays compatible with existing **Swifty**
repositories, reuses **Steam Workshop** content, keeps one **shared mod store**
for everything it downloads, and verifies every byte it installs.

```
nimby install
  @anizay    dir-link  linked   6  store 0  net 0  saved  285.1 MB
  @acre2     dir-link  linked 108  store 0  net 0  saved  637.0 MB
  @melb      overlay   linked   4  store 2  net 2  saved   90.4 MB
saved 1.0 GB (workshop 1.0 GB + store 63 KB), downloaded 63 KB
```

## Why

Swifty repos are plain HTTP folders: `repo.json` plus one `mod.srf` per mod.
Nimby speaks that format exactly (modern JSON `mod.srf` **and** the older text
format), so existing community repos keep working with no server changes. On top
of that Nimby:

- **reuses the Steam Workshop cache** — if a repo mod is byte-identical to an
  item Steam already downloaded, Nimby junctions/hardlinks it instead of
  downloading gigabytes again;
- **keeps a shared store** — files Nimby downloads land once in a folder you
  choose and are linked into every workspace, so a second profile or repo that
  needs the same mod costs zero bytes and zero traffic;
- **verifies every part** it links or downloads (the same MD5 chain Swifty uses);
- **never modifies the Workshop or store folders** — they stay the owner, Nimby
  only links out of them;
- **refuses to clobber** folders it did not create.

Measured on a real repo (92 mods, 610 local Workshop items): 70% of mods map to a
Workshop item, and for those present locally the sampled reuse was **85% of
bytes**, with five full mods (1.5 GB) reused 100% byte-exact.

## Install

Requires Rust 1.80+ (GNU or MSVC host). The GUI additionally needs a .NET SDK.

```powershell
cargo build --release                 # -> target\release\nimby.exe
powershell -File scripts\build.ps1    # same, with a local MinGW toolchain
```

The GUI:

```powershell
dotnet build gui\Nimby.App
gui\Nimby.App\bin\Debug\net10.0\Nimby.App.exe
```

It drives the CLI with `--json`, so anything the GUI does is reproducible on the
command line. Set `NIMBY_CLI` to point at a specific `nimby.exe` if needed.

## GUI

Six pages, all backed by the same CLI:

| Page | What it does |
|---|---|
| **Install** | Active repo banner/icon and its install folder, mod list with checkboxes and a **per-mod progress bar + status** that follows the run (auto-scroll), overall progress bar (byte-exact, updates during each download, shows the current file, how many run in parallel, **live transfer rate and ETA**), install / verify / status, log |
| **Repositories** | Add, remove, switch between repos; shows each repo's own artwork, server list and its subfolder under the workspace root |
| **Configuration** | Arma 3 folder (with autodetect), scratch folder, shared store, workspace, link mode, parallel downloads, BattlEye |
| **Parameters** | Typed editor for launch parameters (flags, text, numbers, choices) with a live command line preview |
| **Stats** | Logical size vs. real bytes on disk per repository, what was saved, and the shared store size |
| **Launch** | Server picker from the repo, BattlEye toggle, redacted command line preview, launch |

Run the headless self-check with `NIMBY_SMOKE=1`: it builds every page, refreshes
it, prints the data it loaded and exits non-zero on failure. `NIMBY_SMOKE=save`
also runs a real `config set` and refreshes every page afterwards, and
`NIMBY_SMOKE_INSTALL=1` additionally installs the smallest mod of the active repo
through the same code path as the Install button (progress events included).

Set `NIMBY_CONFIG=<path>` to use another configuration file, for example for a
portable setup or to keep tests away from your real paths.

## Usage

```powershell
nimby doctor                       # Steam, Arma 3, Workshop, links, store
nimby repo add <url> --name "Community" --select
nimby repo add <url> --name "Community" --folder synixe    # own subfolder
nimby repo folder <url> [folder|--default]                 # show or change it
nimby repo list                                            # url, folder and full path
nimby inspect <url> --images       # summary + cached repo.png / icon.png
nimby install                      # active repo, configured workspace
nimby install --mods "@cba,@ace"
nimby install --optional "@jsrs" --path D:\Alt
nimby install --progress json      # one JSON progress event per line
nimby install --parallel 12        # raise the per-mod download concurrency
nimby install --mod-parallel 3     # install 3 mods at the same time
nimby install --on-error ask --control-stdin   # let the caller answer prompts
nimby install --max-retries 5      # more attempts before giving up
nimby install --no-required --optional "@jsrs"   # skip the required mods
nimby update --check               # quick compare: tracking index vs repo.json
nimby update --check --deep        # also hash every file to catch drift
nimby update                       # fetch the mods that changed
nimby track                        # per-mod breakdown from the tracking index
nimby track --rebuild              # index folders installed before tracking existed
nimby track --verify               # hash everything and list what needs repair
nimby repair                       # reinstall mods whose checksum no longer matches
nimby verify C:\Nimby\@ace         # rehash against the manifest
nimby stats                        # per-repo savings
nimby store info                   # shared store size, mods, files
nimby store prune                  # drop empty folders from the shared store
nimby launch --server Contracts --dry-run
nimby launch --server Contracts --battleye
```

`nimby update --check` compares the workspace against the repository manifests
(what should be installed), while `nimby repair` compares it against the stored
checksums on disk and reinstalls whatever no longer matches.

`nimby migrate` imports the settings of an existing Swifty installation
(`%LOCALAPPDATA%\Swifty\swifty.json` by default, `--source` to point elsewhere),
so the workspace, Arma path, scratch folder and launch parameters carry over.
Use `--dry-run` to see what would be imported first.

Every command accepts `--json`.

### Configuration

Stored at `%APPDATA%\Nimby\config.json`, edited with:

```powershell
nimby config                                    # show, --json for the GUI
nimby config set --arma "E:\SteamLibrary\steamapps\common\Arma 3"
nimby config set --workspace D:\Nimby\mods --shared D:\Nimby\shared
nimby config set --temp D:\Nimby\cache --link-mode overlay --parallel 12
nimby config set --mod-parallel 3
nimby config set --battleye true
nimby config set --clear-params --param '-noSplash' --param '-name=Fred'
```

Arma 3 and Steam are autodetected (registry plus `libraryfolders.vdf`), so
`--arma` is only needed when the game lives somewhere Steam cannot see it.

### Workspace layout

The workspace is a **root**, not a single mod folder: every repository installs
into its own subfolder, so two communities can never overwrite each other and
each can be updated, verified or repaired on its own.

```text
E:\Nimby\                       workspace root
E:\Nimby\Synixe-Contractors\@cba_ws\   one repo's mods
E:\Nimby\Synixe-Contractors\.nimby\    its locks + tracking.json
E:\Nimby\Other-Community\@...          another repo, fully isolated
```

The subfolder name is derived from the repository name (`Synixe Contractors` →
`Synixe-Contractors`), can be renamed per repo (`nimby repo folder`), is always
kept inside the root, and cannot collide with another repository. `install`,
`status`, `launch`, `update`, `repair` and `stats` all work on the active
repository's subfolder.

Folders used before this layout (mods sitting directly in the root) are moved
into the active repository's subfolder with

```powershell
nimby migrate --layout --dry-run   # show the plan
nimby migrate --layout             # rename `@mods` and `.nimby` into the subfolder
```

It renames, so hardlinks and the shared store stay intact — nothing is copied.

### Launch parameters

`-name` is always quoted (`-name="Fred"`), and any value containing spaces is
quoted too. Parameters Nimby sets itself (`-mod`, `-connect`, `-port`,
`-password`) are flagged as managed in `nimby config --json` so the GUI can keep
them out of the picker. Anything else can be typed as a custom parameter.

### Repository tooling

```powershell
nimby srf ".\@mymod" --format v3          # write mod.srf
nimby srf ".\@mymod" --format legacy      # old swifty-cli text format
nimby publish --src .\repo-src --out .\repo-out --name "My Community"
nimby publish --src .\repo-src --out .\repo-out --format legacy
```

`publish` regenerates checksums, copies payloads, writes `repo.json` and both
image checksums, so `repo-out` can be served by any static web server.

## Progress output

`nimby install --progress json` streams one JSON object per line, so a UI can show
an accurate bar measured against the real total byte count. `--progress text`
prints a compact human readable version.

| event | payload |
|---|---|
| `resolving` | `mod`, `index`, `count` — reading `mod.srf` manifests |
| `plan` | `repo`, `mods`, `files`, `totalBytes` |
| `modStart` | `mod`, `index`, `count`, `bytes`, `files`, `percent` |
| `fileStart` | `mod`, `path`, `bytes`, `active`, `doneBytes`, `totalBytes`, `percent` |
| `fileProgress` | `mod`, `path`, `fileBytes`, `active`, `doneBytes`, `totalBytes`, `percent`, `speed`, `etaSeconds` |
| `file` | `mod`, `path`, `action`, `bytes`, `active`, `doneBytes`, `totalBytes`, `doneFiles`, `totalFiles`, `percent`, `speed`, `etaSeconds` |
| `modDone` | `mod`, `mode`, `savedBytes`, `downloadedBytes`, `percent` |
| `note` | `message` — non-fatal warning or a concurrency change |
| `retry` | `mod`, `attempt`, `attempts`, `message`, `delayMs` — a transfer failed and is being retried |
| `resumed` | `mod`, `files`, `bytes` — files from an interrupted run were verified and kept |
| `needsAction` | `mod`, `kind`, `message`, `options` — the CLI is **blocked** until you answer |
| `skipped` | `mod`, `message` — the mod was skipped on request |
| `cancelled` | `installed`, `pending`, `doneBytes`, `totalBytes`, `percent`, `resumable` |
| `failed` | `mod`, `message` |
| `done` | `mods`, `doneBytes`, `totalBytes`, `savedBytes`, `downloadedBytes`, `percent`, `averageSpeed`, `elapsedMs` |

`action` is one of `workshop` (linked from the Steam cache), `store` (linked from
the shared store), `download` (fetched), `copy`, or `reuse` (already correct from
an interrupted run). Dir-linked mods emit a single
`file` event with `path` = `(whole mod)`, so the bar advances by the whole mod at
once.

`speed` is the current transfer rate in bytes per second (smoothed over a ~4 s
window, 0 when nothing is arriving) and `etaSeconds` is the time left at that
rate; `averageSpeed` on `done` is the mean over the whole run. `fileStart` fires
before a file is touched and `fileProgress` roughly every 1 MiB (at most ~50 times
per file), so a single multi-hundred-MB PBO still moves the bar. `doneBytes` is the sum of finished files plus the partial bytes of the
files in flight, and `active` reports how many downloads are running, so
`percent` is always exact.

## Cancelling and resuming

`nimby install --control-stdin` reads one control word per line from stdin:

| word | effect |
|---|---|
| `cancel` | stop before the next file; a running transfer stops between chunks |
| `retry` / `skip` / `abort` | answer a `needsAction` event |

A cancel is cooperative: the current chunk (8 MiB) finishes, then the install
stops with exit code **3** and `cancelled` on stdout. Everything already
verified stays on disk, including partially downloaded payloads in the shared
store, so pressing Install again

1. keeps every file that still matches its checksum (`resumed` event, `reuse`
   actions), instead of downloading it again,
2. resumes partially transferred files from the parts that verify, and
3. prunes files the manifest no longer lists.

Verified on a 476 MB mod: cancel at 6 %, re-run finished with 39 MB of extra
traffic instead of 476 MB. The GUI exposes this as the **Cancel** button; the
status line then says "Install again to resume — finished files are kept".

## Errors, retries and prompts

Every transfer is retried automatically before the user is bothered: transient
failures (timeouts, resets, 5xx, 429, truncated or corrupt bodies) back off
250 ms → 4 s, up to `--max-retries` (3 by default). Permanent failures (404,
missing source, bad manifest) are reported immediately.

After that the behaviour is chosen with `--on-error`:

| mode | behaviour |
|---|---|
| `abort` (default) | stop, print the error, exit **1** |
| `retry` | keep retrying, capped at `2 × --max-retries` manual attempts |
| `skip` | leave the mod as is and continue; the run ends with exit **4** |
| `ask` | emit `needsAction` and wait for `retry`/`skip`/`abort` on stdin |

`--on-error ask` without `--control-stdin` behaves like `abort`, so scripts and
CI never hang. The GUI always uses `ask`: it shows the reason, offers
**Retry / Skip mod / Stop**, and flashes the taskbar button (the only thing that
works when the window is in the background) until the decision is made.

Exit codes: `0` complete, `1` failed, `3` cancelled (resumable), `4` incomplete
(some mods skipped).

## Parallel downloads

Missing files are fetched in concurrent waves. The worker count starts
conservatively (max 4) and adapts: it measures how much of each transfer's wall
time was spent writing to disk and

- **lowers** the count when writes dominate (> 55 % of the time), so a slow disk
  is not hammered and the machine stays responsive,
- **raises** it when the network is the bottleneck (< 20 %), up to the configured
  maximum.

Two consecutive samples in the same direction are required, so it does not
oscillate. Changes are announced with a `note` event. The ceiling comes from
`config.parallelDownloads` (default 8) and can be overridden per run with
`nimby install --parallel N`.

Measured on a 2000-file mod behind a local HTTP server: 16.8 s sequential vs
13.8 s adaptive, with the limiter climbing 4 → 8 and the UI staying responsive.

### Parallel mods

Mods are installed in parallel too: `parallelMods` (`--mod-parallel`, default 2)
workers drain a queue, and each active mod keeps the full per-file download
budget. The GUI shows every mod's own bar, so it is obvious which ones are
running.

Measured on 5 mods (138 MB total, cold store):

| parallel mods | wall time |
|---|---|
| 1 | 6.2 s |
| 2 | **5.2 s** |
| 3 | 5.6 s |
| 5 | 8.5 s |

Giving each mod fewer workers was slower (dividing 8 slots by 3 dropped the run to
9.5 s), because chunk transfers are what saturates the link — so the budget is per
active mod, not global, and `parallelMods` stays low.

### Chunked transfers

Swifty repos split large PBOs into thousands of contiguous parts. Fetching them
as individual requests wastes a round trip per part, so Nimby coalesces
contiguous parts into 8 MiB chunks and runs up to 4 chunk workers per file on top
of the adaptive file-level limiter. If the server ignores `Range`, Nimby falls
back to a single streaming request per file. Part files are kept when a transfer
fails, so the next run resumes instead of restarting.

Result on a real repo (`@cba_ws`, `@cba_rf`, `@pulau`, cold cache):

| approach | wall time |
|---|---|
| one request per part, sequential | 78 s |
| 8 MiB chunks, 4 workers, adaptive files | **8.4 s** |

### Checksum cache

Hashing dominates warm runs, so results are memoised in
`%APPDATA%\Nimby\hashcache.json` keyed by path, size and mtime. `install` and
`verify` both read and update it; a reinstall of an unchanged mod skips hashing
entirely, and edited files are rehashed automatically. `nimby update --check`
stays fast because of this.

## Tracking index

Every install folder keeps `<workspace>/.nimby/tracking.json`: one entry per mod,
one record per file with

| field | meaning |
|---|---|
| `origin` | `download` (fetched from the repo), `copy`, `workshop`, `store` or `reuse` |
| `location` | where the bytes live: `workspace`, `store` or `workshop` |
| `link` | `hardlink`, `symlink`, `junction`, `copy` or `existing` |
| `bytes`, `checksum` | what the file should be |
| `size`, `mtime` | stat signature at install time |
| `source` | the workshop/store path or repository-relative path it came from |

That index does three jobs:

1. **Accurate stats.** Unique (downloaded) bytes, shared-store bytes and Workshop
   bytes are counted per file instead of guessed from aggregate counters, so the
   Stats page can show `unique / shared / workshop / workspace` per repository
   and per mod.
2. **Fast update checks.** `nimby update --check` compares the indexed checksum
   with `repo.json` and stats every file — no hashing, no per-mod `mod.srf` round
   trip. A repository with thousands of files answers in milliseconds.
3. **Deep checks on demand.** `nimby update --check --deep` (or `nimby track
   --verify`) hashes the files that changed and reports `outdated` (the repo moved
   on), `drifted` (local bytes differ), `missing`, or `untracked`.

`nimby track` prints the breakdown per mod, `--json` includes the per-file check
result, and `nimby repair` uses the same data to reinstall only the files that are
actually wrong. The index is written after every mod, so an interrupted or
cancelled run still leaves usable data behind.

Folders installed before tracking existed can be indexed retroactively with
`nimby track --rebuild`: it reads the lock files, stats every file and infers the
origin (`workshop:`, `store:` or `download:`), so no mod has to be reinstalled.

## Statistics

`nimby stats` (and the Stats page) reports, per repository:

| column | meaning |
|---|---|
| logical | bytes the mods would take if every file were a real copy |
| on disk | real footprint Nimby owns: shared-store payload + workspace copies |
| workspace | bytes the `@mod` folders themselves hold (small when files are linked) |
| downloaded | unique bytes fetched from the repository (what only this machine needed) |
| saved | bytes linked from the Steam cache or the store instead of being written again |

A repository that is entirely hardlinked from a 25 GB store therefore shows a
large **on disk** and a nearly empty **workspace** — the store payload is real
disk usage, it is just shared. Workshop bytes are not counted in **on disk**
because Steam owns them; they appear in **saved** instead. The store is counted
once per file, so the totals stay honest when several workspaces share it.

## Install modes

Chosen automatically per mod, overridable with `--mode`:

| Mode | When | Result | Extra disk |
|---|---|---|---|
| `dir-link` | every file matches a Workshop item or the store | junction/symlink `@mod` to that folder | 0 |
| `overlay` | some files match | per-file hardlinks plus fetch for the rest | only the rest |
| `copy` | links impossible (exFAT, network share) | copies bytes | full |
| `passthrough` | opt-in | launch straight from the source folder | 0 |

Windows uses junctions for folders and hardlinks for files, so no administrator
rights are needed. Symlinks are the fallback (cross-volume or non-NTFS), then
copy.

## Layout

```
<workspace>\@cba_ws\             mod tree (real files and/or links)
<workspace>\.nimby\@cba_ws.json  provenance: repo, checksum, mode, sources, stats
<shared>\@cba_ws\addons\...      one copy of each downloaded file
%APPDATA%\Nimby\config.json      paths, link mode, parameters, repos
%APPDATA%\Nimby\images\<hash>\   cached repo.png / icon.png per repository
```

The `.nimby` lock entry stores the full manifest, so `verify`, `status` and
`stats` work offline and even for directory-linked mods whose folder belongs to
Steam.

## Safety

- Repo paths are validated: absolute paths, `..`, drive prefixes, UNC paths,
  reserved Windows device names and control characters are rejected before any
  filesystem access.
- Every downloaded byte range is hashed before it is written into place; a
  mismatch aborts that mod's install.
- Downloads are written through a temporary file and renamed, so an interrupted
  transfer never leaves a corrupt payload in the shared store.
- Nothing is written while Arma 3 runs.
- Passwords are redacted from `--dry-run` output and logs.
- Nimby never executes repository content.

## Format reference

The verified on-the-wire format, including the exact checksum chain, is in
[docs/FORMAT.md](./docs/FORMAT.md).

## Repository layout

```
crates/nimby-core   formats, hashing, PBO reader, installer, linker, store,
                    Steam, config, launch parameters, stats, graphics
crates/nimby-cli    the `nimby` executable
gui/Nimby.App       Avalonia desktop UI (drives the CLI via --json)
docs                format reference
PLAN.md             roadmap, risks and research notes
```
