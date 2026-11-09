# Swifty repository format (verified)

Everything below was reverse-checked against a live repository
(`https://cdn.synixe.contractors/repo/main/`, Swifty client 3.4.1, repo
`version` 3.2.0.0) and a local Arma 3 install. Values are reproduced by the test
suite in `crates/nimby-core`.

## 1. HTTP layout

```
<base>/repo.json            repository manifest
<base>/repo.png             optional repo banner (imageChecksum = SHA1)
<base>/icon.png             optional repo icon   (iconImageChecksum = SHA1)
<base>/@mod/mod.srf         per-mod manifest
<base>/@mod/<path>          payload, mirrors the on-disk mod tree
```

Servers may support `Range` (`206 Partial Content`); when they do not, Nimby
falls back to a full `GET`. Paths are percent-encoded per segment.

## 2. `repo.json`

```json
{
  "repoName": "Synixe Contractors",
  "checksum": "BA3E594DA3DFBD2EED6627637B88404F64640EE0",
  "clientParameters": "-skipIntro -noSplash -noPause",
  "version": "3.2.0.0",
  "requiredMods": [{ "modName": "@cba_ws", "checkSum": "DD39F3C34318C8CB5950E0B785FBCB08", "enabled": true }],
  "optionalMods": [],
  "servers": [
    { "name": "Contracts", "address": "arma.synixe.contractors", "port": "2302", "password": "…", "battleEye": false }
  ]
}
```

Rules implemented by Nimby:

- property names are matched case-insensitively (`checkSum` / `checksum`);
- unknown properties survive a load/save cycle (`basePath`, `iconImagePath`,
  `repoImagePath`, image checksums, client-only fields);
- `checksum` is 40 hex characters. The algorithm is **not** public; Nimby treats
  it as opaque and omits it when publishing, which the client tolerates.

## 3. `mod.srf`

Two formats exist. Nimby auto-detects by the first non-whitespace byte (`{` =
JSON, otherwise legacy text).

### 3.1 Modern (Swifty 3, JSON, UTF-8 **with BOM**)

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
        { "Path": "$$HEADER$$", "Length": 207,   "Start": 0,     "Checksum": "BD24FBB68EF67070E883C54995F32543" },
        { "Path": "config.cpp", "Length": 10909, "Start": 289,   "Checksum": "2CBA6F17C3C5AAD66E34C4EAA2977051" },
        { "Path": "$$END$$",    "Length": 21,    "Start": 60472, "Checksum": "36716380731836FB34C66998EDE6A2CA" }
      ]
    }
  ]
}
```

- `Type`: `SwiftyFile` or `SwiftyPboFile`.
- Parts are contiguous and cover the whole file:
  `Start[0] == 0`, `Start[n] == Start[n-1] + Length[n-1]`,
  `Σ Length == Length` (verified on a 388-file mod).
- For PBOs the parts are `$$HEADER$$`, one part per PBO entry, then `$$END$$`
  (the trailing 21-byte signature block) when the PBO carries one.
- Part paths are informational; bytes are always fetched as file ranges.
- Paths use `/` in JSON.

### 3.2 Legacy (swifty-cli text)

```
ADDON:<modName>:<fileCount>:<modHash>
PBO:<path>:<size>:<partCount>:<fileHash>
<partPath>:<start>:<size>:<partHash>
FILE:<path>:<size>:<partCount>:<fileHash>
<partPath>:<start>:<size>:<partHash>
```

`path` uses `\`. `ADDON` comes first, each `PBO`/`FILE` line is followed by its
part lines. A `mod.srf` inside the folder is skipped when generating.

## 4. Checksum chain (all MD5, uppercase hex, ASCII input)

1. **Part** `= MD5(file bytes [Start, Start+Length))`.
   Bytes are taken as stored — a compressed PBO part is hashed compressed.
2. **File** `= MD5(concat(part.Checksum))` over the parts in order.
   Example: `MD5("32DCD6F6EB27CF18E4917106356ADCED") = 0A266973E65CDA6F39BC5C140AC937FC`.
3. **Mod (v3)** `= MD5(concat(file.Checksum + file.Path))` with files sorted
   ordinal by `Path`.
4. **Mod (legacy)** `= MD5(concat(file.Checksum[0..16]))` with files sorted by
   path — the swifty-cli behaviour.

Golden values reproduced by `crates/nimby-core/src/checksum.rs` tests:

| Mod | v3 checksum |
|---|---|
| `@cba_ws` | `DD39F3C34318C8CB5950E0B785FBCB08` |
| `@cba_rf` | `5AF55C4096FF52B52CC3F7106A04ACA1` |
| `@pulau`  | `0272F923FD8CC7C82BA2966722A71E9C` |

`repo.json.requiredMods[i].checkSum` equals the mod's `mod.srf` root checksum.

## 5. PBO layout (needed only when writing manifests)

```
00 's' 'r' 'e' 'V'         magic
4 × u32                    version / reserved
(name, value)*             property list, NUL-terminated; ends with an empty name
(name, 5 × u32)*           entries: method, original size, offset, timestamp, data size
empty name + 5 × zero u32  terminator
<data>                     concatenated entry data (offset 0 = sequential)
[21 bytes]                 optional signature trailer: 1 flag byte + 20-byte SHA1
```

`method == 0` means stored; `0x56657273` (`Vers`) marks the terminator entry.
Verified on 32 stock Workshop PBOs. Data size sum plus trailer equals the file
size.

## 6. Local client state (for migration)

`%APPDATA%\Swifty\Settings.json` holds `armaPath`, `addonsPaths`, `tempPath`,
`parallelDownloads` and one object per repository: `url`, `path`, `parameters`,
`mods[] {modName, checkSum, enabled}`, `servers[]`, `requiredDLCS[]` plus a
`modCaches[]` index of `{path, modifiedDateTimeUtc}` per file, which the client
uses to skip rehashing unchanged files.

## 7. Steam / Arma 3 facts

- Workshop content: `steamapps\workshop\content\107410\<publishedid>\`.
- Manifest data: `steamapps\workshop\appworkshop_107410.acf`
  (`WorkshopItemsInstalled.<id>.{size, timeupdated, manifest, ugchandle}`).
- A repo mod's `meta.cpp` contains `publishedid = <id>;` when it is a Workshop
  re-upload, giving an exact repo-mod → Workshop-item mapping (70% of the
  sampled repo).
- The Arma 3 Launcher itself exposes Workshop items as junctions under
  `Arma 3\!Workshop\@Name`, so junctions/symlinks are safe for Arma — including
  for mods that ship DLLs.
- Executables: `arma3_x64.exe`, `arma3.exe`, `arma3battleye.exe` (BE launcher),
  `arma3launcher.exe`, `arma3server_x64.exe`.
- Command line: `-mod=<p1>;<p2>;…` with absolute paths, `-connect=`, `-port=`,
  `-password=`. Windows caps the command line at 8191 characters, so very large
  repos need a short workspace root.
