# Contributing to Nimby

Thanks for taking the time to help. This file explains how to build the project,
what a good change looks like, and how to get it merged.

## Before you start

- **Bugs and feature ideas** — open an issue. For install problems, the most useful
  report includes `nimby doctor --json`, `nimby status --json`, the repository URL
  and the mod names involved.
- **Security problems** — do not open a public issue. See [SECURITY.md](SECURITY.md).
- **Big changes** — describe the plan in an issue first. Nimby has a small surface
  and a lot of behaviour that is hard to guess (link modes, resume, the checksum
  chain); an issue saves you from building the wrong thing.

## Layout

```
crates/nimby-core/     repository format, HTTP, checksums, Workshop discovery,
                       link layer, installer, tracking index, config
crates/nimby-cli/      the `nimby` binary: clap commands over nimby-core
gui/Nimby.App/         Avalonia desktop app; it drives `nimby.exe --json`,
                       it never re-implements install logic
docs/                  FORMAT.md (repository format), TUTORIAL.md, screenshots
scripts/               build.ps1, package.ps1, make-icon.ps1
assets/branding/       product icon and author mark (brand assets, see NOTICE)
```

## Build

Prerequisites: Rust stable, .NET 10 SDK, and a C toolchain for `ring` — either MSVC
build tools or MinGW-w64.

```powershell
powershell -File scripts\build.ps1                    # debug build
powershell -File scripts\build.ps1 build --release
dotnet build gui\Nimby.App -c Release
```

`scripts/build.ps1` passes every argument to cargo. It finds a MinGW-w64 toolchain
on `PATH`, in a few common locations, or via `NIMBY_MINGW`; `NIMBY_TMP` moves the
build scratch directory. With the MSVC toolchain it does nothing special.

Both binaries must end up in one folder tree: the GUI looks for `bin\nimby.exe`, then
`nimby.exe` next to itself, then `<repo>\target\{release,debug}\nimby.exe`, then `PATH`.
`NIMBY_CLI` overrides the lookup. A CLI started from a `bin` folder shares the `data`
folder of its parent, which is how the portable release keeps one configuration.

## Test

```powershell
powershell -File scripts\build.ps1 fmt --all --check
powershell -File scripts\build.ps1 clippy --all-targets -- -D warnings
powershell -File scripts\build.ps1 test
```

The suite is offline: format fixtures cover `repo.json`, `mod.srf` v3 and legacy
`mod.srf`, the checksum chain, PBO headers, path-traversal fuzzing, link creation,
install modes, shared-store dedup, progress accounting, adaptive concurrency, config
round trips, parameter rendering and hash caching. Golden values come from the live
Synixe repository and are recorded in the tests.

### GUI self-checks

The Avalonia app has headless modes used by CI. Each one exits non-zero on failure
and prints a short log.

| Variable | What it exercises |
|---|---|
| `NIMBY_SMOKE=1` | walks every page, checks the CLI is found and the layout renders |
| `NIMBY_SMOKE=save` | the above plus a settings round trip |
| `NIMBY_SMOKE_INSTALL=1` | a real install of the smallest mod in the active repo (needs network) |
| `NIMBY_SMOKE_CANCEL=1` | starts that install, cancels it, verifies the resume path |
| `NIMBY_SMOKE_SHOTS=<dir>` | renders pages to PNG, used to refresh `docs/screenshots` |

Use a throwaway `NIMBY_CONFIG` (for example a file under `%TEMP%`) when running
these so the checks never touch your real configuration.

## Guidelines

- **Match the surrounding code.** Small, focused functions, `anyhow` in the CLI and
  typed errors in the core, no unnecessary dependencies.
- **Never regress the reuse paths.** A change that makes Nimby download bytes it
  could hardlink from the Workshop or the shared store is a bug.
- **Keep the CLI and the GUI in step.** The GUI reads `--json` output; changing a
  JSON field is a breaking change for the UI, so update both sides and the docs.
- **Say what you measured.** Performance claims belong in a comment or the PR body
  with the number and how you got it.
- **Comment the non-obvious.** Why a link mode is chosen, why a race needed a
  particular lock — not a restatement of the code.
- **Docs are code.** `README.md`, `docs/FORMAT.md`, `docs/TUTORIAL.md` and the CLI
  help must keep matching the behaviour you change.

## Commits and pull requests

- One logical change per commit, present tense, imperative subject
  (`install: keep verified files when a transfer is cancelled`).
- In the PR body: what changed, why, how it was tested, and any user-visible effect.
- CI must be green: `fmt`, `clippy -D warnings`, `test`, the release build and the
  GUI smoke test.
- New behaviour needs a test. Fixes need a regression test when the bug is
  reproducible offline.

## Licence

Nimby is licensed under **GPL-3.0-or-later**. By opening a pull request you agree
that your contribution is licensed under the same terms. The brand assets in
`assets/branding/` are not part of that licence — do not add third-party artwork,
name badges or logos to them without permission.
