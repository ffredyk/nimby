# Nimby — plan

Modern Swifty-compatible Arma 3 addon manager.

Goals:

1. Read + write existing Swifty repos (no server changes needed).
2. Reuse Arma 3 Workshop cache. Link files, do not copy, when content identical.
3. Fast, verifiable, safe updates. Delta download when possible.

## Implementation status (v0.2)

Done and verified against live data:

- `nimby-core`: repo.json + `mod.srf` (v3 JSON **and** legacy text) parse/write,
  MD5 checksum chain, PBO reader, HTTP source with Range + fallback + basic auth,
  verifier, installer with `auto` mode selection (`dir-link`/`overlay`/`copy`/
  `passthrough`), **byte-level progress events and adaptive parallel downloads**,
  **shared mod store** (download once, hardlink everywhere),
  Windows junction + hardlink layer, Steam discovery (registry,
  `libraryfolders.vdf`, `appworkshop_107410.acf`, `meta.cpp`), **persisted
  config** (paths, link mode, repos, launch parameters), **typed launch
  parameter model** (with `-name` quoting), **per-repo stats**, **repo artwork
  cache**, launcher with password redaction, `.nimby/*.json` provenance.
- `nimby-cli`: `srf`, `verify`, `publish`, `inspect`, `install`, `update`,
  `repair`, `migrate`, `store info|prune`, `status`, `stats`, `doctor`,
  `launch`, `config [set]`, `repo add|remove|select|list`, all with `--json`.
- **Chunked parallel transfers**: contiguous PBO parts are merged into 8 MiB
  range requests with up to 4 workers per file on top of the adaptive file-level
  limiter (`78 s → 8.4 s` on `@cba_ws,@cba_rf,@pulau`), streaming fallback when
  the server ignores `Range`, and part-level resume on failure.
- **Persistent hash cache** (`%APPDATA%\Nimby\hashcache.json`, path+size+mtime →
  checksum, rayon-parallel) shared by `install` and `verify`, so warm runs skip
  rehashing entirely.
- **Maintenance commands**: `update [--check]`, `repair`, `migrate [--source]
  [--dry-run]` (imports a real Swifty settings file), `store info|prune`,
  `--no-required` for optional-only installs, and a command-line length guard for
  `-mod=` (8191 characters).
- **Fault tolerance**: transient transfers are retried with exponential backoff
  (`--max-retries`), permanent failures can `abort`/`retry`/`skip`/`ask`
  (`--on-error`), and `ask` blocks on a `needsAction` event until the GUI answers
  over stdin (`--control-stdin`). The GUI flashes the taskbar button instead of
  stealing focus, then offers Retry / Skip mod / Stop.
- **Cancel and resume**: `cancel` stops between chunks, keeps every verified file
  and partial payload, and the next run reuses them (`reuse` actions, `resumed`
  event, stale files pruned). Verified: cancel at 6 % of a 476 MB mod, resume
  finished with 39 MB of traffic instead of 476 MB.
- **Per-chunk file handles**: download workers open the payload themselves, so a
  cloned handle cannot share a file pointer and interleave writes (that race
  produced sporadic "range assembly" checksum failures after a resume).
- **Per-repository tracking index** (`<folder>/.nimby/tracking.json`): every file
  records its origin (`download`/`copy`/`workshop`/`store`/`reuse`), its location
  (`workspace`/`store`/`workshop`), the link kind, the expected checksum and a
  size+mtime signature. It drives exact stats (`unique` vs `shared` vs
  `workshop`), a millisecond `update --check`, `update --check --deep` for drift
  and `nimby track`.
- **Workspace root with one subfolder per repository**: `workspacePath` is a
  root, `RepoEntry.folder` names the isolated subfolder inside it (derived from
  the repo name, renameable, never colliding, never escaping the root);
  `install`, `status`, `launch`, `update`, `repair` and `stats` follow the active
  repository's subfolder, and `migrate` turns Swifty's per-repo path into the
  root.
- **Parallel mods**: `parallelMods` / `--mod-parallel` drains a queue with N
  workers (each mod keeps the full per-file budget; fewer per-mod workers was
  measurably slower). Measured on 138 MB across 5 mods: 6.2 s with one mod at a
  time, 5.2 s with two, 8.5 s with five.
- **Speed and ETA in the progress stream**: `speed` / `etaSeconds` on every
  `fileProgress` and `file` event (smoothed 4 s window, drops to 0 when idle) and
  `averageSpeed` / `elapsedMs` on `done`; the GUI shows `23,6 MB/s · ~18s left`
  next to the overall bar and the rate on the active mod row.
- **Per-mod install view**: every row in the mod list shows its own progress bar,
  percentage, file counter and state (queued / fetching / downloaded / reused /
  retry / needs a decision / done / skipped), and the list scrolls to follow the
  mod being worked on. Rows are fed from an ingest-side state map so a `modDone`
  can never be overtaken by a coalesced `file` event.
- `scripts/build.ps1`, `scripts/package.ps1` and `.github/workflows/ci.yml` for
  reproducible builds, a portable zip and CI.
- `gui/Nimby.App`: Avalonia UI with six pages (Install, Repositories,
  Configuration, Parameters, Stats, Launch), repo artwork, folder pickers,
  parameter editor with live command line preview, byte-exact install progress
  (coalesced so thousands of files never flood the UI), and `NIMBY_SMOKE`
  headless page/install self-check.
- 82 Rust tests: checksum golden vectors from the live repo, format round trips,
  path traversal fuzz, PBO header parsing, link create/remove, install modes,
  shared-store dedup, progress accounting, adaptive concurrency, config round
  trip, parameter rendering, stats aggregation, command-line limit, hash cache,
  failed-install cleanup.

Live proof:

- `nimby publish` of Workshop-sourced `@cba_ws` reproduces the official
  checksum `DD39F3C34318C8CB5950E0B785FBCB08`.
- `nimby install` of `@anizay` + `@acre2` linked 922 MB from the Workshop cache,
  downloaded 0 bytes.
- A second workspace on the same machine installed the same mods with
  **0 downloaded bytes** — everything came from Workshop plus the shared store.
- A throttled 20 MB file streamed live progress to the GUI
  (`0% → 10% → 15% → … → 100%`, `2 in parallel`).
- A 2000-file mod installed through the GUI in ~20 s without freezing; adaptive
  concurrency climbed 4 → 8 workers.
- Cached image SHA1 values match the ones the Swifty client stores
  (`6CFFF06E…`, `26CE73F6…`).
- The GUI cancelled a 476 MB install at 6 % ("Install again to resume"), and the
  re-run finished from the kept bytes.
- Tracking caught a tampered file: `update --check` reported `drifted` (1 file),
  `repair` refetched 126 B and the index was updated.
- `nimby migrate` imported a live Swifty 3.4.1 install (Arma path, `E:\TEMP`
  scratch folder, `E:\Synixe2` workspace, 1 repo, 3 parameters,
  `parallelDownloads=10`).
- `nimby update --check` and `nimby repair` are quiet on an unchanged workspace;
  `nimby store info|prune` reports and trims the shared store correctly.

Not done yet: Steam drift detection/repair loop, repo signing, end-to-end
validation against the real Swifty client.

---

## 1. Goals / non-goals

In scope:

- Client: add repo, verify, repair, update, launch (server connect, BattlEye).
- Link engine: Workshop → mod dir via junction / symlink / hardlink.
- Repo tooling: `new`, `publish` (Swifty-compatible), `validate`.
- Migration: import `%APPDATA%\Swifty\Settings.json`.
- GUI (Avalonia) after CLI works.

Out of scope v1:

- Own HTTP protocol / own repo format. Swifty format is the contract.
- Rewriting PBOs. Nimby never edits PBO content.
- Signing / trust chain. Nimby adds optional repo signing later, never breaks Swifty compat.

---

## 2. Compat contract (verified facts)

### 2.1 Repo layout (HTTP)

```
<base>/repo.json
<base>/repo.png              optional
<base>/icon.png              optional
<base>/@mod/mod.srf          required, per mod
<base>/@mod/<any path>       mod payload, mirrors disk tree
```

Live test: `https://cdn.synixe.contractors/repo/main/repo.json` → 200, `Accept-Ranges: bytes`, `Range` → `206`.

So: byte-range download works. Must still fall back to full GET when server ignores Range.

### 2.2 repo.json (client version 3.4.1, repo `version` 3.2.0.0)

```json
{
  "repoName": "Synixe Contractors",
  "checksum": "BA3E...40 hex...",
  "requiredMods": [{ "modName": "@cba_ws", "checkSum": "DD39F3C34318C8CB5950E0B785FBCB08", "enabled": true }],
  "optionalMods": [],
  "clientParameters": "-skipIntro -noSplash -noPause",
  "version": "3.2.0.0",
  "servers": [{ "name": "Contracts", "address": "...", "port": "2302", "password": "...", "battleEye": false }]
}
```

Legacy fields (old swifty-cli docs) must parse too: `basePath`, `iconImagePath`, `repoImagePath`, `imageChecksum`.

Rules:

- Case-insensitive property names.
- Unknown fields → keep, do not fail.
- `checksum` top level: 40 hex (SHA1 shape). Algorithm unknown. Not required for client work. Task M0-4.
- `checkSum` per mod == `mod.srf` root checksum.

### 2.3 mod.srf — two formats

**Modern (JSON, UTF-8 WITH BOM, PascalCase):**

```json
{
  "Name": "@cba_ws",
  "Checksum": "DD39F3C34318C8CB5950E0B785FBCB08",
  "Files": [
    {
      "Path": "addons/lxws_cba_compat.pbo",
      "Length": 60493,
      "Checksum": "5670E8F32A56599E68CAD0ECD83B2380",
      "Type": "SwiftyPboFile",
      "Parts": [
        { "Path": "$$HEADER$$", "Length": 207, "Start": 0, "Checksum": "BD24FBB68EF67070E883C54995F32543" },
        { "Path": "config.cpp", "Length": 10909, "Start": 289, "Checksum": "2CBA6F17C3C5AAD66E34C4EAA2977051" },
        { "Path": "$$END$$", "Length": 21, "Start": 60472, "Checksum": "36716380731836FB34C66998EDE6A2CA" }
      ]
    }
  ]
}
```

Type values seen: `SwiftyFile`, `SwiftyPboFile`.

**Legacy (text, from swifty-cli docs):**

```
ADDON:<name>:<fileCount>:<hash>
PBO:<path>:<size>:<partCount>:<hash>     (or FILE:...)
<partName>:<start>:<size>:<hash>
```

Path separators in legacy text: `\`. Modern JSON: `/`.

### 2.4 Hash algorithms (verified against live data)

All MD5, all uppercase hex, ASCII.

1. **Part**: `MD5(raw bytes of file[Start .. Start+Length))`.
   Verified on `mod.cpp`, `.bikey`, `.pbo` (PBO parts are raw byte ranges, not decompressed).
2. **File**: `MD5(ascii(concat(part.Checksum)))`.
   Verified: `MD5("32DCD6F6...")` == file checksum for `mod.cpp`.
3. **Mod (root)**: `MD5(ascii(concat(file.Checksum + file.Path)))`, files sorted ordinal by `Path`.
   Verified on `@cba_ws`, `@pulau`, `@cba_rf`, `@ace` (4/4 match, no others matched).
4. Parts are contiguous: `Start[0] == 0`, `Start[n] == Start[n-1] + Length[n-1]`, `sum(Length) == file.Length`. Verified on 388-file mod.

Consequence: verification and delta download need no PBO parsing for the JSON format. PBO parse only needed to **write** parts (part names = PBO entries) and legacy srf.

### 2.5 Swifty client local state (for migration)

`%APPDATA%\Swifty\Settings.json`:

```
version, armaPath, addonsPaths, tempPath, parallelDownloads,
width, height, copyTeamSpeak,
repositories[] {
  name, url, username, password, path, parameters,
  checksum, imageChecksum, iconImageChecksum, autocheck, lastUpdated,
  servers[], mods[] { modName, checkSum, enabled },
  addtionalMods[], optionalMods[], requiredDLCS[],
  modCaches[] { addonName, folderPath, addonChecksum, folderModifieDateTimeUtc,
                files[] { path, modifiedDateTimeUtc } }
}
```

Note `modCaches`: client trusts size/mtime cache, rehashes only when mtime changed. Same trick Nimby uses.

Client install: `%LOCALAPPDATA%\Swifty3` (Velopack, WPF, `app-3.4.1`). Logs in `%APPDATA%\Swifty\logs`.

### 2.6 Arma 3 / Steam facts (verified on this machine)

- Game root example: `E:\SteamLibrary\steamapps\common\Arma 3`.
- Executables: `arma3_x64.exe`, `arma3.exe`, `arma3battleye.exe` (BE launcher, resolves `_x64` service), `arma3launcher.exe`, `arma3server_x64.exe`.
- Workshop content: `steamapps\workshop\content\107410\<publishedid>\`.
- Workshop manifest: `steamapps\workshop\appworkshop_107410.acf` → `WorkshopItemsInstalled.<id>.{size,timeupdated,manifest,ugchandle}`.
- Workshop mod root: `addons\*.pbo`, `addons\*.pbo.<key>.bisign`, `keys\*.bikey`, `mod.cpp`, `meta.cpp`, `userconfig\`.
- `meta.cpp` has `publishedid = <id>;` → direct repo-mod → workshop-item mapping.
- Arma 3 Launcher builds `!Workshop\@Name` **junctions** pointing at workshop dirs. Proves Arma tolerates junctions/symlinks (also for DLL mods).

### 2.7 Reuse potential (measured against live repo + this machine)

- Local workshop items: 610.
- Repo mods: 92. With `meta.cpp` `publishedid`: 64 (70%). Already present locally: 32.
- 14 locally-present mods sampled: 3973 MB of 4651 MB had matching file sizes (85%).
- 5 mods hash-verified fully: **100% byte-exact** reuse (`@acre2` 668 MB, `@anizay` 299 MB, `@drakovac` 361 MB, `@ihantalaw` 131 MB, `@melb` 95 MB).
- Exception: `@ace` repo build differs from Workshop build (PBOs rebuilt) → only small files (bisign/bikey) reusable.

So: linking saves real GB. Not every mod. Must be opportunistic with exact verification.

---

## 3. Architecture

```
Nimby.Core          formats, hashing, PBO reader, planner, linker, index
Nimby.Sources       SwiftyHttp, LocalDir, SteamWorkshop
Nimby.Cli           Spectre.Console/System.CommandLine app (nimby)
Nimby.App           Avalonia GUI
Nimby.RepoTools     new/publish/validate (Swifty-cli parity)
Nimby.Tests         unit + golden + integration
```

.NET 9 (or 10) LTS. Core targets `net9.0` (cross-platform for Linux servers/CI). Linker has Windows + POSIX implementations.

### 3.1 Core objects

- `RepoManifest` — repo.json.
- `ModManifest` — mod.srf (modern/legacy unified model).
- `ModFile` / `FilePart`.
- `Workspace` — install root (repo `path`).
- `Source` — where bytes come from: `HttpSource`, `LocalSource`, `WorkshopSource`.
- `FileKey` — `(volumeId, fileId, size, mtimeUtc)` → cached hash.
- `InstallRecord` — provenance for every mod: mode, source, checksum, file map.
- `Plan` — ordered actions: `Keep`, `LinkDir`, `LinkFile`, `Download`, `Delete`, `Repair`.

### 3.2 State store

Implemented as flat JSON, not SQLite: it is easy to inspect, diff and repair by
hand, and the volumes involved are small.

- `%APPDATA%\Nimby\config.json` — paths, link mode, battleye, launch parameters
  and the repo list (`src/config.rs`).
- `<workspace>\.nimby\@mod.json` — per-mod provenance: repo URL, checksum, mode,
  per-file source label, full `mod.srf` copy and byte accounting
  (`src/workspace.rs`).
- `<shared>\@mod\...` — the shared store payloads (`src/store.rs`).
- `%APPDATA%\Nimby\images\<sha1(url)>\repo.png|icon.png` — cached artwork
  (`src/graphics.rs`).

A SQLite hash index (`(volume, file id, size, mtime) -> md5`) remains a possible
optimisation for very large workspaces; today hashing is fast enough because only
mismatching files are rehashed.

### 3.3 Flow

```
resolve repo → fetch repo.json (etag) → for each mod fetch mod.srf (etag)
  → build desired state
  → scan actual state (lstat: link? reparse point? size/mtime)
  → index workshop candidates
  → planner: per file choose source (workshop / repo / local store)
  → executor: materialize, verify parts, write install record
  → launch preflight: re-verify cheap keys, detect Steam drift
```

---

## 4. Workshop reuse design

### 4.1 Candidate discovery

Order (cheap → expensive):

1. `meta.cpp` `publishedid` in repo mod → workshop dir. Best signal.
2. `mod.cpp` `name` / folder name similarity → candidate set.
3. Content fingerprint: index of workshop mod file paths+sizes; find item with max overlap.

### 4.2 Matching rules

- Candidate must contain every repo-only path OR we go overlay mode (§4.3).
- Size check first (`Length` from srf). Cheap reject.
- Part-hash check for final proof. Cache by `(size, mtime)` so repeat checks are free.
- Never trust name, never trust size alone.

### 4.3 Materialization modes (per mod, auto-chosen, overridable)

Both the Workshop cache and the shared store are "roots" the planner may link
from; Workshop wins when both have the bytes, so provenance stays accurate.

| Mode | When | How | Disk |
|---|---|---|---|
| `passthrough` | all files byte-exact, no extra repo files | launch `-mod=<root>` directly | 0 |
| `dir-link` | whole dir matches (Workshop item or store mod dir) | junction (Win) / symlink (POSIX) `@mod` → root | 0 |
| `overlay` | partial match | dir `@mod` with per-file hardlink/symlink to the root; fetch the rest | extra only |
| `copy` | no link possible (network FS, exFAT, cross-volume symlink denied) | copy bytes | full |

Downloads always land in the shared store when one is configured (written through
a temporary file, then renamed), and are hardlinked into the workspace. A later
install of the same mod therefore costs zero traffic and zero extra bytes, and a
fully-populated store also enables `dir-link`.

Rules:

- Windows dirs: junction first (no admin, local volumes). Symlink only if cross-volume and privilege available.
- Windows files: hardlink first (no admin, same volume). Symlink if privilege. Copy otherwise.
- POSIX: symlink.
- `passthrough` off by default (Steam can update under you). Enable via `--workshop=passthrough`.
- Symlink privilege check: try create in temp, cache result. If denied → degrade, tell user why.

### 4.4 Drift / Steam updates

- Track `workshop_items.{manifest,time_updated}` from ACF + per-file index.
- Before launch: fast scan (size/mtime) of linked files vs install record. Drift → plan repair.
- Arma running → block repair (`arma3*.exe` process check).
- Hardlink hazard: if Steam writes in place, linked copy changes silently. Mitigations:
  1. Detect ACF `timeupdated`/`manifest` change → mark mod dirty.
  2. `nimby verify --deep` rehashes.
  3. Optional `--link=snapshot` stores hardlinked copy in Nimby store and re-checks source manifest; on change, relink from `local_hashes` cache or re-download.
- Workshop item deleted by user → broken links. Detect (target missing) → repair via download.
- Mod dirs never written by Nimby while Arma runs. Atomic: build in `.nimby-tmp`, then rename into place.

### 4.5 Never link

- `mod.srf` itself. Always ours.
- Any file whose path escapes the mod root (`..`, absolute, drive-qualified, reserved names). Reject at parse time.
- Symlinks inside workshop dirs pointing outside the item (resolve target, must stay inside item root).

---

## 5. Installer layout

```
<workspace>\@mod\...          mod tree (real files / links)
<workspace>\.nimby\lock.json  per-mod provenance (human readable, diffable)
<workspace>\.nimby\tmp\       staging
```

`lock.json` per mod:

```json
{
  "repo": "https://.../repo/main/",
  "mod": "@ace",
  "checksum": "…",
  "mode": "overlay",
  "source": { "kind": "workshop", "publishedid": "463939057", "manifest": "-1", "timeupdated": 123 },
  "files": { "addons/x.pbo": { "kind": "link", "from": "...", "md5": "…" } }
}
```

---

## 6. CLI surface

Implemented today:

```
nimby repo add <url> [--name N] [--user U] [--pass P] [--select]
nimby repo list|remove <url>|select <url>
nimby config [--json]                     # full snapshot for the GUI
nimby config set [--arma P] [--temp P] [--shared P] [--workspace P]
                 [--link-mode auto|dir-link|overlay|copy|passthrough]
                 [--parallel N] [--battleye BOOL]
                 [--clear-params] [--param EXPR]...
nimby install [URL] [--path P] [--mods a,b] [--optional a,b] [--mode M]
              [--no-workshop] [--no-store] [--no-required] [--force] [--parallel N]
nimby update [URL] [--path P] [--check]
nimby repair [URL] [--path P] [--mods a,b]
nimby migrate [--source P] [--dry-run]
nimby store info|prune [--json]
nimby verify <dir> [--srf P] [--strict] [--json]
nimby status [--path P] [--json]
nimby stats [--path P] [--json]            # per-repo logical / on disk / saved
nimby inspect <url> [--mods a,b] [--images] [--json]
nimby doctor [--json]
nimby launch [--arma P] [--path P] [--repo URL] [--server NAME] [--battleye]
             [--param EXPR]... [--dry-run]
nimby srf <dir> [--format v3|legacy] [--out P]
nimby publish --src DIR --out DIR [--config P] [--name N] [--mods a,b] [--format v3|legacy]
```

Planned: `new` repo wizard.

Exit codes stable. `--json` for GUI/CI. No interactive prompts.

Launch args (verified pieces):

```
arma3_x64.exe  (or arma3battleye.exe when BE on)
-mod=<p1>;<p2>;...   absolute, quoted if spaces
-connect=<addr> -port=<port> -password=<pw>
<repo clientParameters> <user params>
```

Command line limit ~8191 chars. 92 mods with long root overflows. Mitigate: short workspace root default (`C:\Nimby`), warn + suggest, or `passthrough`/junction to shrink. Measure in `launch --dry-run`.

---

## 7. GUI (phase 3)

Avalonia. Screens:

- Repo picker (URL, auth, image from repo).
- Mod list: required / optional toggles, size, "Workshop linked" badge, per-mod reason when not linked.
- Progress: per-file part download, link vs download counters.
- Disk report: saved GB, linked file count.
- Launch bar: server dropdown (from repo.json servers), BE toggle, params.
- Settings: paths, parallel downloads, link mode, Steam library overrides.

Reuse Core only. MVVM (CommunityToolkit.Mvvm).

---

## 8. Security

- Treat repos as untrusted:
  - Reject absolute paths, `..`, UNC, drive-relative, reserved device names, `:` in names, control chars.
  - Canonicalize target; must stay under mod root.
  - Do not follow symlinks when creating dirs (no TOCTOU link swap).
- HTTPS required by default; `--allow-http` opt-in.
- Basic auth supported (Swifty settings use it).
- Verify every downloaded part hash before commit. Never mark installed on partial failure.
- Atomic file writes (temp file + rename). Rollback on error.
- No secrets in logs. Redact `password`/auth.
- No remote code execution. Nimby never runs repo content.
- Optional future: Ed25519 signed repos (sidecar file), opt-in.

---

## 9. Performance

Implemented:

- **Chunked parallel HTTP.** Contiguous PBO parts coalesce into 8 MiB ranges;
  up to 4 chunk workers per file, plus an adaptive file-level limiter (default
  ceiling 8, `parallelDownloads`) that measures the share of wall time spent
  writing and steps down when disk-bound (> 55 %) and up when network-bound
  (< 20 %), with two-sample hysteresis.
- **Streaming fallback** when the server ignores `Range`, so progress stays live.
- **Resume.** Partial downloads keep their `.part` file and their verified
  ranges, so a retry only fetches what is missing.
- **Hashing.** 1 MB sequential reads, MD5 only (compat), rayon-parallel across
  files, results memoised by `(path, size, mtime)` in
  `%APPDATA%\Nimby\hashcache.json`.
- **Workshop / store lookups** use a path+size index built once per run, and
  files with a matching checksum are hardlinked instead of downloaded.
- `update --check` short-circuits on the cached checksums, so an unchanged
  10 GB workspace answers in well under a second.

Measured: `@cba_ws,@cba_rf,@pulau` cold install 78 s → 8.4 s; 2000-file mod
16.8 s sequential → 13.8 s adaptive (local server, disk-bound).

---

## 10. Testing

Unit + golden:

- Golden vectors from live repo (hardcode the verified checksums above).
- Round-trip: write srf → parse → same checksums.
- Hash fixture matrix: 0-byte file, >4 GiB file, unicode names, deep paths, legacy text srf, BOM/no-BOM.
- PBO parser: uncompressed, CWS compressed, `$PBOPREFIX$`, property entries, `$$END$$` 21-byte checksum.
- Linker: junction, symlink, hardlink, cross-volume fallback, privilege-denied fallback, dangling target.
- Path traversal fuzz on srf paths.
- HTTP: fake server with/without Range, flaky server, truncated body, 416.

Integration:

- Two real repos (own + Synixe).
- Full install with Workshop reuse, then `verify --deep` must be clean.
- Simulate Steam update (touch + change ACF) → drift detected → repair.
- Arma launch with BE on/off, signature check against a `verifySignatures=2` test server.

CI: Windows + Linux (core). Linker tests skip on non-matching FS.

---

## 11. Milestones

| # | Deliverable | Est | Status |
|---|---|---|---|
| M0 | Format spec + golden tests. Resolve repo-level `checksum`. Confirm client honors pre-linked files (test with real client). | 2-3 d | spec + goldens done; repo `checksum` still unknown; real-client test outstanding |
| M1 | Core: repo.json/model, srf v3+legacy parse, hashing, PBO reader | 1 wk | done |
| M2 | HTTP client: range download, resume, verification, fallback | 1 wk | done (8 MiB chunks, 4 workers/file, part resume, streaming fallback) |
| M3 | Workspace, planner, install record, verify/repair | 1-2 wk | done (`update`, `repair`, hash cache) |
| M4 | Link engine + Workshop index + matching + drift | 1-2 wk | links + index + matching done; drift detection outstanding |
| M5 | Launcher + preflight + servers + BE | 3-5 d | done (command-line length guard included) |
| M6 | CLI polish, `migrate --from-swifty`, `doctor` | 1 wk | done (`migrate`, `store info` / `store prune`, `--no-required`) |
| M7 | Repo tools: new/publish/validate (both srf formats) | 1 wk | `publish`/`srf`/`inspect --images` done; `new` wizard outstanding |
| M8 | GUI (Avalonia) | 2-3 wk | done (six pages, live progress, repo artwork); cancel button outstanding |
| M9 | Hardening, perf, docs, packaging (Velopack/Squirrel), auto-update | 1-2 wk | perf + docs + `scripts/package.ps1` + CI done; auto-update and signing outstanding |

Order matters: M0 blocks everything. M4 is the differentiator.

---

## 12. Risks / open questions

1. **Client rehash behavior.** If Swifty client ignores mtime cache and rehashes, links still pass (bytes identical) but startup cost rises. Test M0.
2. **`repo.json.checksum` algorithm unknown.** If client uses it to decide "repo changed", our generator may cause extra refetch. Test M0.
3. **Steam in-place writes.** Could corrupt hardlinked files. Mitigation §4.4. Consider defaulting to `dir-link` for full matches (junction target replaced by Steam = link follows, safe) over per-file hardlinks.
4. **Passthrough safety.** Workshop updates can break server compatibility mid-session. Off by default.
5. **Repack gap.** Some communities rebuild PBOs (ACE case) → less reuse. Communicate per-mod.
6. **Long paths.** Arma is not long-path safe. Short root, validation.
7. **Non-NTFS targets.** exFAT/network → copy mode.
8. **Antivirus.** Hashing big trees may be slow; cached index helps.
9. **Old repos.** Legacy text srf + legacy repo.json fields; need one real sample to test. Ask community for old repo.
10. **Arma with junctioned DLL mods.** Launcher already does it; still verify BE + signature with a real server.

---

## 13. First sprint (concrete)

1. `dotnet new sln` + projects: Core, Cli, Tests.
2. Implement `RepoManifest`, `ModManifest` (v3 + legacy), hashing, with golden tests using the verified constants.
3. Implement PBO reader (header + entries) for part generation; test against `addons/*.pbo` from workshop.
4. Implement `nimby validate` (hash a mod dir, compare srf/repo.json).
5. Spike: point a mod dir at workshop files via junctions, run Swifty client, confirm "no update needed".
6. Write format spec doc from §2.
