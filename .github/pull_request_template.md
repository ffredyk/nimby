## What changed

<!-- One or two sentences. Link the issue this closes: "Closes #12". -->

## Why

<!-- The bug, the limitation or the measurement that motivated this. -->

## How it was tested

- [ ] `cargo fmt --all --check`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test`
- [ ] GUI smoke: `NIMBY_SMOKE=1` (add `NIMBY_SMOKE_INSTALL=1` / `NIMBY_SMOKE_CANCEL=1` when install paths changed)
- [ ] Manual check with a real repository: <!-- which repo, which mods, what you saw -->

## Effect

- User-visible behaviour: <!-- none / describes -->

## Checklist

- [ ] Reuse paths are untouched or improved — nothing that could be linked from the Workshop or the shared store is now downloaded.
- [ ] CLI and GUI stay in sync (`--json` field changes are reflected in the UI and the docs).
- [ ] `README.md` / `docs/FORMAT.md` / `docs/TUTORIAL.md` updated where behaviour changed.
- [ ] No credentials, personal paths or local `D:\...` build paths in the diff.
- [ ] I agree my contribution is licensed under GPL-3.0-or-later and adds no third-party brand assets.
