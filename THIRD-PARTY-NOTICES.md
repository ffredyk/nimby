# Third-party notices

Nimby is licensed under the GNU General Public License v3.0 or later (see [LICENSE](LICENSE)).
It is built with the open-source components listed below. Each component stays under its own licence;
nothing here changes the terms of the GPL for Nimby itself.

The complete licence texts ship in the [LICENSES](LICENSES) folder of this repository and of every
release archive. The lists below were generated from the committed `Cargo.lock` and
`cargo tree --workspace --edges normal --target x86_64-pc-windows-gnu`, so they describe exactly the
code linked into the released binaries.

## Application framework (GUI)

| Component | Version | Licence | Project |
|---|---|---|---|
| .NET runtime and base class library | 10.0 | MIT | https://github.com/dotnet/runtime |
| Avalonia | 12.1.5 | MIT | https://github.com/AvaloniaUI/Avalonia |
| Avalonia.Desktop | 12.1.5 | MIT | https://github.com/AvaloniaUI/Avalonia |
| Avalonia.Themes.Fluent | 12.1.5 | MIT | https://github.com/AvaloniaUI/Avalonia |
| Avalonia.Fonts.Inter | 12.1.5 | MIT | https://github.com/AvaloniaUI/Avalonia |
| Inter typeface (bundled by Avalonia.Fonts.Inter) | 4.x | SIL Open Font License 1.1 | https://github.com/rsms/inter |
| AvaloniaUI.DiagnosticsSupport (development builds only) | 2.2.3 | MIT | https://github.com/AvaloniaUI/Avalonia |

The published release is self-contained, so the .NET runtime is redistributed inside it under the MIT
licence. The Inter font is embedded under the SIL Open Font License 1.1
([LICENSES/OFL-1.1.txt](LICENSES/OFL-1.1.txt)) and is not modified.

## Rust crates used directly

| Crate | Version | Licence | Used for | Project |
|---|---|---|---|---|
| `anyhow` | 1.0.104 | MIT OR Apache-2.0 | error context | https://crates.io/crates/anyhow |
| `thiserror` | 2.0.21 | MIT OR Apache-2.0 | error types | https://crates.io/crates/thiserror |
| `serde` | 1.0.229 | MIT OR Apache-2.0 | JSON de/serialisation | https://crates.io/crates/serde |
| `serde_json` | 1.0.154 | MIT OR Apache-2.0 | repo.json, mod.srf, tracking | https://crates.io/crates/serde_json |
| `md-5` | 0.10.6 | MIT OR Apache-2.0 | the Swifty checksum chain | https://crates.io/crates/md-5 |
| `sha1` | 0.10.7 | MIT OR Apache-2.0 | legacy repository signatures | https://crates.io/crates/sha1 |
| `hex` | 0.4.3 | MIT OR Apache-2.0 | checksum formatting | https://crates.io/crates/hex |
| `clap` | 4.6.7 | MIT OR Apache-2.0 | CLI argument parsing | https://crates.io/crates/clap |
| `reqwest` | 0.12.28 | MIT OR Apache-2.0 | HTTP transfers over rustls | https://crates.io/crates/reqwest |
| `walkdir` | 2.5.0 | Unlicense OR MIT | workspace scanning | https://crates.io/crates/walkdir |
| `rayon` | 1.12.0 | MIT OR Apache-2.0 | parallel hashing | https://crates.io/crates/rayon |
| `dirs` | 5.0.1 | MIT OR Apache-2.0 | profile paths | https://crates.io/crates/dirs |
| `tempfile` | 3.27.0 | MIT OR Apache-2.0 | atomic downloads | https://crates.io/crates/tempfile |
| `url` | 2.5.8 | MIT OR Apache-2.0 | repository URLs | https://crates.io/crates/url |
| `winreg` | 0.52.0 | MIT | Steam and Arma discovery in the registry | https://crates.io/crates/winreg |
| `windows-sys` | 0.59.0 | MIT OR Apache-2.0 | hardlinks, junctions, reparse points | https://crates.io/crates/windows-sys |
| `libc` | 0.2.191 | MIT OR Apache-2.0 | symlinks in non-Windows builds | https://crates.io/crates/libc |

## Rust crates linked transitively

159 crates are linked into `nimby.exe` and the core library. Grouped by the licence expression
declared in each crate's `Cargo.toml`:

**MIT / Apache-2.0** — 89 packages

```
anstream 1.0.0
anstyle 1.0.14
anstyle-parse 1.0.0
anstyle-query 1.1.5
anstyle-wincon 3.0.11
anyhow 1.0.104
async-compression 0.4.52
base64 0.22.1
base64 0.23.1
bitflags 2.13.2
block-buffer 0.10.4
cfg-if 1.0.5
clap 4.6.7
clap_builder 4.6.7
clap_derive 4.6.7
clap_lex 1.1.1
colorchoice 1.0.5
compression-codecs 0.4.47
compression-core 0.4.34
cpufeatures 0.2.17
cpufeatures 0.3.1
crc32fast 1.5.4
crossbeam-deque 0.8.8
crossbeam-epoch 0.9.21
crossbeam-utils 0.8.23
crypto-common 0.1.7
digest 0.10.7
dirs 5.0.1
dirs-sys 0.4.1
displaydoc 0.2.7
either 1.19.0
flate2 1.1.11
form_urlencoded 1.2.2
futures-channel 0.3.34
futures-core 0.3.34
futures-io 0.3.34
futures-sink 0.3.34
futures-task 0.3.34
futures-util 0.3.34
getrandom 0.2.17
getrandom 0.4.3
hashbrown 0.17.1
heck 0.5.0
hex 0.4.3
http 1.5.0
httparse 1.10.1
idna 1.1.0
ipnet 2.12.2
is_terminal_polyfill 1.70.2
itoa 1.0.18
libc 0.2.191
log 0.4.35
md-5 0.10.6
once_cell 1.21.4
once_cell_polyfill 1.70.2
percent-encoding 2.3.2
proc-macro2 1.0.107
quote 1.0.47
rayon 1.12.0
rayon-core 1.13.0
reqwest 0.12.28
rustls-pki-types 1.15.1
serde 1.0.229
serde_core 1.0.229
serde_derive 1.0.229
serde_json 1.0.154
sha1 0.10.7
smallvec 1.16.5
socket2 0.6.5
stable_deref_trait 1.2.1
syn 2.0.119
syn 3.0.9
tempfile 3.27.0
thiserror 1.0.69
thiserror 2.0.21
thiserror-impl 1.0.69
thiserror-impl 2.0.21
tokio-rustls 0.26.6
typenum 1.20.1
url 2.5.8
windows-link 0.2.1
windows-sys 0.48.0
windows-sys 0.52.0
windows-sys 0.59.0
windows-sys 0.61.2
windows-targets 0.48.5
windows-targets 0.52.6
windows_x86_64_gnu 0.48.5
windows_x86_64_gnu 0.52.6
```

**Apache-2.0 / MIT** — 10 packages

```
atomic-waker 1.1.2
equivalent 1.0.3
fastrand 2.5.0
idna_adapter 1.2.2
indexmap 2.14.2
pin-project-lite 0.2.17
utf8_iter 1.1.0
utf8parse 0.2.2
utf_types 1.0.0
zeroize 1.9.1
```

**MIT / Apache-2.0** — 1 package

```
serde_urlencoded 0.7.1
```

**MIT** — 23 packages

```
bytes 1.12.1
generic-array 0.14.7
http-body 1.1.0
http-body-util 0.1.5
hyper 1.12.0
hyper-util 0.1.21
mio 1.2.4
simd-adler32 0.3.10
slab 0.4.12
strsim 0.11.1
synstructure 0.14.0
tokio 1.54.1
tokio-util 0.7.20
tower 0.5.3
tower-http 0.6.11
tower-layer 0.3.3
tower-service 0.3.3
tracing 0.1.44
tracing-core 0.1.36
try-lock 0.2.5
want 0.3.2
winreg 0.52.0
zmij 1.0.23
```

**Apache-2.0** — 1 package

```
sync_wrapper 1.0.2
```

**Apache-2.0 OR ISC OR MIT** — 2 packages

```
hyper-rustls 0.27.10
rustls 0.23.46
```

**Apache-2.0 AND ISC** — 1 package

```
ring 0.17.14
```

**ISC** — 2 packages

```
rustls-webpki 0.103.16
untrusted 0.9.0
```

**Unicode-3.0** — 18 packages

```
icu_collections 2.4.0
icu_locale_core 2.4.0
icu_normalizer 2.4.0
icu_normalizer_data 2.4.0
icu_properties 2.4.0
icu_properties_data 2.4.0
icu_provider 2.4.0
litemap 0.8.3
potential_utf 0.1.6
tinystr 0.8.5
writeable 0.6.4
yoke 0.8.4
yoke-derive 0.8.4
zerofrom 0.1.8
zerofrom-derive 0.1.8
zerotrie 0.2.5
zerovec 0.11.9
zerovec-derive 0.11.7
```

**(MIT OR Apache-2.0) AND Unicode-3.0** — 1 package

```
unicode-ident 1.0.27
```

**Unlicense / MIT** — 2 packages

```
memchr 2.8.3
winapi-util 0.1.11
```

**Unlicense / MIT** — 2 packages

```
same-file 1.0.6
walkdir 2.5.0
```

**0BSD OR MIT OR Apache-2.0** — 1 package

```
adler2 2.0.1
```

**BSD-3-Clause** — 1 package

```
subtle 2.6.1
```

**Zlib** — 1 package

```
foldhash 0.2.0
```

**MIT OR Zlib OR Apache-2.0** — 1 package

```
miniz_oxide 0.9.1
```

**Apache-2.0 OR BSL-1.0** — 1 package

```
ryu 1.0.23
```

**MPL-2.0** — 1 package

```
option-ext 0.2.0
```

**CDLA-Permissive-2.0** — 1 package

```
webpki-roots 1.0.11
```

## Licence texts

| Licence | File |
|---|---|
| GNU General Public License v3.0 | [LICENSE](LICENSE) |
| Apache License 2.0 | [LICENSES/Apache-2.0.txt](LICENSES/Apache-2.0.txt) |
| MIT | [LICENSES/MIT.txt](LICENSES/MIT.txt) |
| ISC | [LICENSES/ISC.txt](LICENSES/ISC.txt) |
| BSD 3-Clause | [LICENSES/BSD-3-Clause.txt](LICENSES/BSD-3-Clause.txt) |
| zlib | [LICENSES/Zlib.txt](LICENSES/Zlib.txt) |
| Mozilla Public License 2.0 | [LICENSES/MPL-2.0.txt](LICENSES/MPL-2.0.txt) |
| SIL Open Font License 1.1 | [LICENSES/OFL-1.1.txt](LICENSES/OFL-1.1.txt) |
| Unicode License v3 | [LICENSES/Unicode-3.0.txt](LICENSES/Unicode-3.0.txt) |
| CDLA-Permissive-2.0 | [LICENSES/CDLA-Permissive-2.0.md](LICENSES/CDLA-Permissive-2.0.md) |

Where a crate offers a choice of licences, Nimby relies on the MIT or the Apache-2.0 option, so those
two texts cover most of the tree. Four components deserve a note:

- **`ring` 0.17.14** (TLS cryptography, reached through rustls) is Apache-2.0 AND ISC and also carries
  third-party notices for the BoringSSL/OpenSSL-derived code it embeds; those notices ship inside the
  crate under `third_party/`. `ring` is linked statically.
- **`option-ext` 0.2.0** is MPL-2.0. It is used unmodified, so the MPL's file-level copyleft is
  satisfied by pointing at its published source, and MPL-2.0 is compatible with the GPL through its
  secondary-licence clause.
- **`webpki-roots` 1.0.11** is CDLA-Permissive-2.0, a permissive data licence for the Mozilla root
  store, with no copyleft effect.
- **ICU4X crates** (the `icu_*`, `zerovec` and `yoke` family, pulled in through `idna`) are
  Unicode-3.0, a permissive licence from the Unicode Consortium.

All of the above are compatible with distributing Nimby under GPL-3.0-or-later.

## Trademarks

Bohemia Interactive, Arma 3 and Steam are trademarks of their respective owners. Nimby is an
independent project, not affiliated with Bohemia Interactive, Valve or the Swifty team.
