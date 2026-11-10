# Changelog

All notable changes to Nimby are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.9.0] — 2026-11-09

First public release. A complete Arma 3 addon manager for existing Swifty
repositories, shipping as a portable Windows application (GUI plus CLI).

### Added

- **Swifty repository compatibility** — reads and writes `repo.json` and
  `mod.srf` (v3 JSON and the legacy text format), verifies the MD5 checksum
  chain and reproduces official mod checksums.
- **Workshop reuse** — content that already exists in the Steam Workshop cache
  is linked into the workspace instead of downloaded: a full match becomes a
  directory link, a partial match an overlay of hardlinks plus the missing
  files. Discovery uses the registry, `libraryfolders.vdf`, `appworkshop_107410.acf`
  and `meta.cpp`.
- **Shared mod store** — download a file once, hardlink it into any number of
  repositories and workspaces.
- **Per-repository workspace subfolders** — the configured workspace is a root,
  each repository gets its own isolated folder that survives renames.
- **Tracking index** — `<folder>/.nimby/tracking.json` records every installed
  file with its origin (download, copy, workshop, store, reuse), location, link
  kind, expected checksum and a size+mtime signature. It powers exact statistics,
  a millisecond-level `update --check` and a hashing `--deep` mode.
- **Parallel chunked downloads** — 8 MiB range requests, up to four workers per
  file, adaptive file-level parallelism tuned to disk and network pressure, and
  MODs installed in parallel (`parallelMods`, default 2).
- **Byte-exact progress** — per-mod progress bars, transfer rate, ETA, current
  file, retries and per-mod state, with auto-scroll in the mod list; install runs
  keep the UI responsive.
- **Cancel and resume** — cancelling keeps every verified file, the next run
  reuses it and only fetches what is missing.
- **Error handling** — transient failures retry with exponential backoff;
  permanent ones can abort, retry, skip or ask, and the GUI flashes the taskbar
  button instead of stealing focus.
- **GUI** — six pages (Install, Repositories, Configuration, Parameters, Stats,
  Launch) plus About, repository artwork, folder pickers, a launch-parameter
  editor with `-name` quoting and a live command-line preview, and the 8191
  character command-line guard.
- **Statistics** — logical size, bytes on disk, bytes in the workspace, bytes
  downloaded and bytes saved, per repository and in total.
- **Maintenance** — `update [--check] [--deep]`, `repair`, `track`, `verify`,
  `store info|prune`, `status`, `doctor`, `stats`, `migrate`.
- **Swifty migration** — `nimby migrate` imports the Swifty client's settings
  (Arma path, scratch folder, workspace, repositories, launch parameters);
  `--layout` moves root-level mods into the repository subfolder using renames
  only, so links survive.
- **Portable configuration** — configuration and the hash cache live in `data\`
  next to the executable when that folder is writable, falling back to
  `%APPDATA%\Nimby`. `NIMBY_CONFIG` overrides both.
- **Licensing and attribution** — GPL-3.0-or-later with additional terms under
  section 7(b) that require every derivative to keep the copyright notice, the
  ffredyk credit and the repository link in its About/Credits screen, version
  output and documentation; `NOTICE`, `LICENSE-ADDITIONAL-TERMS.md`,
  `THIRD-PARTY-NOTICES.md` with licence texts in `LICENSES/`, an About page with
  the website and source links, and a credit line in the GUI and in
  `nimby doctor`.

### Fixed

- The GUI could launch copies of itself instead of the installer: Windows resolves
  `nimby.exe` to `Nimby.exe` in the same folder, so the CLI lookup now skips the
  running executable, and the CLI no longer inherits the GUI smoke switches.
- Per-mod progress no longer resets to zero when the mod list refreshes after an
  install completes.

### Performance

- Cold-cache install of `@cba_ws`, `@cba_rf` and `@pulau` went from 78 s with
  sequential per-part requests to 8.4 s with chunked parallel transfers.
- 138 MB across five mods: 6.2 s with one mod at a time, 5.2 s with two.
- A 2000-file mod installs through the GUI in about 20 s; adaptive concurrency
  climbs from 4 to 8 workers.
- Resume after cancelling at 6 % of a 476 MB mod transferred 39 MB instead of
  476 MB.

### Known limitations

- Windows 10/11 x64 only; the GUI is Avalonia, the CLI also builds on other
  platforms but is only tested there for repository, verify and publish work.
- The builds are not code-signed, so SmartScreen may warn on first launch.
- Symlink creation may need Developer Mode or elevation; Nimby prefers junctions
  and hardlinks, which do not.
- Arma 3 repository signing and the Swifty client protocol are out of scope.

[0.9.0]: https://github.com/ffredyk/nimby/releases/tag/v0.9.0
