# Screenshots

These pages are rendered by the app itself: `NIMBY_SMOKE_SHOTS=<folder>` starts
the GUI, visits every page and saves its own rendered frame as PNG.

The repository shown in them is a **fictional demo repository** (name, artwork
and server list are placeholders served from `127.0.0.1`) so the images do not
promote any real Arma 3 community. Mod names use the `@example_*` prefix and the
payload is generated test data.

To refresh them, point `NIMBY_CONFIG` at a config with at least one installed
repository, then run:

```powershell
$env:NIMBY_CONFIG = 'D:\Nimby\config.json'
$env:NIMBY_SMOKE_SHOTS = 'D:\Nimby\shots'
gui\Nimby.App\bin\Release\net10.0\Nimby.exe
```

Each page is captured at 1120x780. Copy the PNGs into this folder under the
names below:

| File | Page |
| --- | --- |
| `01-install.png` | Install |
| `02-repos.png` | Repositories |
| `03-config.png` | Configuration |
| `04-parameters.png` | Parameters |
| `05-stats.png` | Stats |
| `06-launch.png` | Launch |
| `07-about.png` | About |
