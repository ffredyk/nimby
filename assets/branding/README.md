# Brand assets

These files are **not** covered by the GPL. See [../NOTICE](../NOTICE) for the full
statement.

| File | What it is | Used by |
|---|---|---|
| `nimby-icon.jpg` | master render of the Nimby product icon (760×760) | source for `scripts/make-icon.ps1` only |
| `nimby-512.png` | rounded, transparent-corner version of the master | README hero |
| `nimby-256.png` | 256 px variant | GUI window icon (`Assets/nimby-256.png`) |
| `nimby.ico` | 7-size Windows icon (256, 128, 64, 48, 32, 24, 16) | `Nimby.exe` icon, Explorer, taskbar |
| `ffredyk-logo.png` | master of the author mark | source for `scripts/make-icon.ps1` only |
| `ffredyk-128.png` | 128 px author mark | About page, README footer |
| `ffredyk-64.png` | 64 px author mark | GUI assets |

## Regenerating the derived files

Everything except the two masters is generated; nothing is hand-edited:

```powershell
powershell -File scripts\make-icon.ps1
```

The script only needs Windows PowerShell and `System.Drawing`, writes the same bytes
for the same input, and is safe to re-run.

## Building without the brand assets

The app icon and the marks are referenced from exactly three places:

1. `gui/Nimby.App/Nimby.App.csproj` — `<ApplicationIcon>` and the four
   `AvaloniaResource` entries.
2. `gui/Nimby.App/MainWindow.axaml` and `gui/Nimby.App/Views/AboutView.axaml` — the
   `avares://` image sources.
3. `scripts/package.ps1` — the staged `assets\branding` folder.

A downstream build that wants to ship Nimby without the marks can substitute its own
images at those paths. The GPL grants that freedom over the source code; it does not
grant the marks themselves.
