use std::fs;
use std::path::Path;

use serde_json::{json, Map, Value};

use crate::checksum::{file_checksum, mod_checksum, mod_checksum_legacy};
use crate::error::{Error, Result};
use crate::pbo;
use crate::util::{get_ci, get_ci_bool, get_ci_string, get_ci_u64, rel_path, strip_bom};

pub const SRF_FILE_NAME: &str = "mod.srf";

// ---------------------------------------------------------------------------
// repo.json
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoMod {
    pub mod_name: String,
    pub checksum: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Server {
    pub name: String,
    pub address: String,
    pub port: String,
    pub password: String,
    pub battle_eye: bool,
}

#[derive(Clone, Debug, Default)]
pub struct RepoManifest {
    pub repo_name: String,
    /// Top level repo checksum. Algorithm is not public; kept opaque.
    pub checksum: Option<String>,
    pub client_parameters: String,
    pub version: Option<String>,
    pub required_mods: Vec<RepoMod>,
    pub optional_mods: Vec<RepoMod>,
    pub servers: Vec<Server>,
    /// Fields we do not model yet (legacy basePath, image paths, ...).
    pub extra: Map<String, Value>,
}

impl RepoManifest {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let text = std::str::from_utf8(bytes)
            .map_err(|e| Error::Manifest(format!("repo.json is not UTF-8: {e}")))?;
        let value: Value = serde_json::from_str(strip_bom(text))?;
        Self::from_value(&value)
    }

    pub fn from_value(value: &Value) -> Result<Self> {
        if !value.is_object() {
            return Err(Error::Manifest("repo.json root is not an object".into()));
        }
        let mut manifest = RepoManifest {
            repo_name: get_ci_string(value, "repoName").unwrap_or_default(),
            checksum: get_ci_string(value, "checksum"),
            client_parameters: get_ci_string(value, "clientParameters").unwrap_or_default(),
            version: get_ci_string(value, "version"),
            ..Default::default()
        };

        for (list_name, target) in [("requiredMods", true), ("optionalMods", false)] {
            let Some(list) = get_ci(value, list_name) else {
                continue;
            };
            let Some(items) = list.as_array() else {
                continue;
            };
            for item in items {
                let Some(mod_name) = get_ci_string(item, "modName") else {
                    continue;
                };
                let checksum = get_ci_string(item, "checkSum")
                    .or_else(|| get_ci_string(item, "checksum"))
                    .unwrap_or_default();
                let enabled = get_ci_bool(item, "enabled").unwrap_or(true);
                let entry = RepoMod {
                    mod_name,
                    checksum,
                    enabled,
                };
                if target {
                    manifest.required_mods.push(entry);
                } else {
                    manifest.optional_mods.push(entry);
                }
            }
        }

        if let Some(servers) = get_ci(value, "servers").and_then(|v| v.as_array()) {
            for server in servers {
                manifest.servers.push(Server {
                    name: get_ci_string(server, "name").unwrap_or_default(),
                    address: get_ci_string(server, "address").unwrap_or_default(),
                    port: get_ci_string(server, "port").unwrap_or_default(),
                    password: get_ci_string(server, "password").unwrap_or_default(),
                    battle_eye: get_ci_bool(server, "battleEye").unwrap_or(false),
                });
            }
        }

        if let Some(obj) = value.as_object() {
            for (key, val) in obj {
                let known = [
                    "repoName",
                    "checksum",
                    "clientParameters",
                    "version",
                    "requiredMods",
                    "optionalMods",
                    "servers",
                ];
                if !known.iter().any(|k| k.eq_ignore_ascii_case(key)) {
                    manifest.extra.insert(key.clone(), val.clone());
                }
            }
        }

        Ok(manifest)
    }

    pub fn load(path: &Path) -> Result<Self> {
        Self::parse(&fs::read(path)?)
    }

    pub fn to_json(&self) -> String {
        let required: Vec<Value> = self
            .required_mods
            .iter()
            .map(|m| {
                json!({
                    "modName": m.mod_name,
                    "checkSum": m.checksum,
                    "enabled": m.enabled,
                })
            })
            .collect();
        let optional: Vec<Value> = self
            .optional_mods
            .iter()
            .map(|m| {
                json!({
                    "modName": m.mod_name,
                    "checkSum": m.checksum,
                    "enabled": m.enabled,
                })
            })
            .collect();
        let servers: Vec<Value> = self
            .servers
            .iter()
            .map(|s| {
                json!({
                    "name": s.name,
                    "address": s.address,
                    "port": s.port,
                    "password": s.password,
                    "battleEye": s.battle_eye,
                })
            })
            .collect();

        let mut obj = Map::new();
        obj.insert("repoName".into(), Value::String(self.repo_name.clone()));
        if let Some(checksum) = &self.checksum {
            obj.insert("checksum".into(), Value::String(checksum.clone()));
        }
        obj.insert(
            "clientParameters".into(),
            Value::String(self.client_parameters.clone()),
        );
        if let Some(version) = &self.version {
            obj.insert("version".into(), Value::String(version.clone()));
        }
        obj.insert("requiredMods".into(), Value::Array(required));
        obj.insert("optionalMods".into(), Value::Array(optional));
        obj.insert("servers".into(), Value::Array(servers));
        for (k, v) in &self.extra {
            obj.entry(k.clone()).or_insert_with(|| v.clone());
        }
        serde_json::to_string_pretty(&Value::Object(obj)).unwrap_or_default()
    }

    /// Every mod, required first then optional.
    pub fn all_mods(&self) -> impl Iterator<Item = &RepoMod> {
        self.required_mods.iter().chain(self.optional_mods.iter())
    }

    pub fn mod_by_name(&self, name: &str) -> Option<&RepoMod> {
        self.all_mods()
            .find(|m| m.mod_name.eq_ignore_ascii_case(name))
    }
}

// ---------------------------------------------------------------------------
// mod.srf
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    File,
    Pbo,
}

impl FileKind {
    pub fn srf_type(&self) -> &'static str {
        match self {
            FileKind::File => "SwiftyFile",
            FileKind::Pbo => "SwiftyPboFile",
        }
    }

    pub fn legacy_type(&self) -> &'static str {
        match self {
            FileKind::File => "FILE",
            FileKind::Pbo => "PBO",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilePart {
    pub path: String,
    pub start: u64,
    pub length: u64,
    pub checksum: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModFile {
    pub path: String,
    pub length: u64,
    pub checksum: String,
    pub kind: FileKind,
    pub parts: Vec<FilePart>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SrfFormat {
    /// Modern Swifty 3 JSON (UTF-8 BOM).
    V3,
    /// Legacy swifty-cli text.
    Legacy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModManifest {
    pub name: String,
    pub checksum: String,
    pub files: Vec<ModFile>,
    pub format: SrfFormat,
}

impl ModManifest {
    /// Parse either the v3 JSON form or the legacy text form, auto-detected.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let text = std::str::from_utf8(bytes)
            .map_err(|e| Error::Manifest(format!("mod.srf is not UTF-8: {e}")))?;
        let text = strip_bom(text).trim_start();
        if text.starts_with('{') {
            Self::parse_v3(text)
        } else {
            Self::parse_legacy(text)
        }
    }

    pub fn load(path: &Path) -> Result<Self> {
        Self::parse(&fs::read(path)?)
    }

    pub fn parse_v3(text: &str) -> Result<Self> {
        let value: Value = serde_json::from_str(strip_bom(text))?;
        let name = get_ci_string(&value, "name")
            .ok_or_else(|| Error::Manifest("mod.srf has no Name".into()))?;
        let checksum = get_ci_string(&value, "checksum")
            .ok_or_else(|| Error::Manifest("mod.srf has no Checksum".into()))?;
        let mut files = Vec::new();
        if let Some(list) = get_ci(&value, "files").and_then(|v| v.as_array()) {
            for item in list {
                let path = get_ci_string(item, "path")
                    .ok_or_else(|| Error::Manifest("mod.srf file has no Path".into()))?;
                let kind = match get_ci_string(item, "type").as_deref() {
                    Some(t) if t.eq_ignore_ascii_case("SwiftyPboFile") => FileKind::Pbo,
                    Some(t) if t.eq_ignore_ascii_case("SwiftyFile") => FileKind::File,
                    _ => infer_kind(&path),
                };
                let mut parts = Vec::new();
                if let Some(part_list) = get_ci(item, "parts").and_then(|v| v.as_array()) {
                    for part in part_list {
                        parts.push(FilePart {
                            path: get_ci_string(part, "path").unwrap_or_default(),
                            start: get_ci_u64(part, "start").unwrap_or(0),
                            length: get_ci_u64(part, "length").unwrap_or(0),
                            checksum: get_ci_string(part, "checksum").unwrap_or_default(),
                        });
                    }
                }
                files.push(ModFile {
                    path,
                    length: get_ci_u64(item, "length")
                        .unwrap_or_else(|| parts.iter().map(|p| p.length).sum()),
                    checksum: get_ci_string(item, "checksum").unwrap_or_default(),
                    kind,
                    parts,
                });
            }
        }
        Ok(ModManifest {
            name,
            checksum,
            files,
            format: SrfFormat::V3,
        })
    }

    pub fn parse_legacy(text: &str) -> Result<Self> {
        let mut name = String::new();
        let mut checksum = String::new();
        let mut files: Vec<ModFile> = Vec::new();

        for raw_line in text.lines() {
            let line = raw_line.trim_end_matches('\r').trim();
            if line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix("ADDON:") {
                let mut it = rest.rsplitn(3, ':');
                checksum = it.next().unwrap_or_default().to_string();
                let _count = it.next();
                name = it.next().unwrap_or_default().to_string();
                continue;
            }
            if let Some(rest) = line
                .strip_prefix("PBO:")
                .or_else(|| line.strip_prefix("FILE:"))
            {
                let kind = if line.starts_with("PBO:") {
                    FileKind::Pbo
                } else {
                    FileKind::File
                };
                let mut it = rest.rsplitn(4, ':');
                let hash = it.next().unwrap_or_default().to_string();
                let _part_count = it.next();
                let length = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                let path = it.next().unwrap_or_default().replace('\\', "/");
                files.push(ModFile {
                    path,
                    length,
                    checksum: hash,
                    kind,
                    parts: Vec::new(),
                });
                continue;
            }
            // part line: name:start:size:hash
            let mut it = line.rsplitn(4, ':');
            let hash = it.next().unwrap_or_default().to_string();
            let length = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            let start = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            let part_path = it.next().unwrap_or_default().to_string();
            if let Some(file) = files.last_mut() {
                file.parts.push(FilePart {
                    path: part_path,
                    start,
                    length,
                    checksum: hash,
                });
            }
        }

        if name.is_empty() {
            return Err(Error::Manifest("legacy mod.srf has no ADDON line".into()));
        }
        if checksum.is_empty() {
            checksum = mod_checksum(&files);
        }
        Ok(ModManifest {
            name,
            checksum,
            files,
            format: SrfFormat::Legacy,
        })
    }

    pub fn to_v3_json(&self) -> String {
        let files: Vec<Value> = self
            .files
            .iter()
            .map(|f| {
                let parts: Vec<Value> = f
                    .parts
                    .iter()
                    .map(|p| {
                        json!({
                            "Path": p.path,
                            "Length": p.length,
                            "Start": p.start,
                            "Checksum": p.checksum,
                        })
                    })
                    .collect();
                json!({
                    "Path": f.path,
                    "Length": f.length,
                    "Checksum": f.checksum,
                    "Type": f.kind.srf_type(),
                    "Parts": parts,
                })
            })
            .collect();

        let mut root = Map::new();
        root.insert("Name".into(), Value::String(self.name.clone()));
        root.insert("Checksum".into(), Value::String(self.checksum.clone()));
        root.insert("Files".into(), Value::Array(files));
        let body = serde_json::to_string(&Value::Object(root)).unwrap_or_default();
        format!("\u{feff}{body}")
    }

    pub fn to_legacy_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "ADDON:{}:{}:{}\n",
            self.name,
            self.files.len(),
            self.checksum
        ));
        for file in &self.files {
            out.push_str(&format!(
                "{}:{}:{}:{}:{}\n",
                file.kind.legacy_type(),
                file.path.replace('/', "\\"),
                file.length,
                file.parts.len(),
                file.checksum
            ));
            for part in &file.parts {
                out.push_str(&format!(
                    "{}:{}:{}:{}\n",
                    part.path.replace('/', "\\"),
                    part.start,
                    part.length,
                    part.checksum
                ));
            }
        }
        out
    }

    /// Checksum as the given format defines it.
    pub fn checksum_for(&self, format: SrfFormat) -> String {
        match format {
            SrfFormat::V3 => mod_checksum(&self.files),
            SrfFormat::Legacy => mod_checksum_legacy(&self.files),
        }
    }

    /// Same manifest, but with `checksum` set to what `format` would publish.
    pub fn formatted(mut self, format: SrfFormat) -> Self {
        self.checksum = self.checksum_for(format);
        self.format = format;
        self
    }

    pub fn to_bytes(&self, format: SrfFormat) -> Vec<u8> {
        match format {
            SrfFormat::V3 => self.to_v3_json().into_bytes(),
            SrfFormat::Legacy => self.to_legacy_text().into_bytes(),
        }
    }

    pub fn write_to(&self, dir: &Path, format: SrfFormat) -> Result<()> {
        fs::write(dir.join(SRF_FILE_NAME), self.to_bytes(format))?;
        Ok(())
    }

    pub fn total_size(&self) -> u64 {
        self.files.iter().map(|f| f.length).sum()
    }
}

fn single_part(path: &Path, length: u64) -> Result<Vec<FilePart>> {
    let checksum = crate::checksum::md5_range(path, 0, length)?;
    let file_name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    Ok(vec![FilePart {
        path: format!("{file_name}_{length}"),
        start: 0,
        length,
        checksum,
    }])
}

fn infer_kind(path: &str) -> FileKind {
    if path.to_ascii_lowercase().ends_with(".pbo") {
        FileKind::Pbo
    } else {
        FileKind::File
    }
}

/// Build a manifest by hashing every file under `dir`.
pub fn generate_from_dir(name: &str, dir: &Path) -> Result<ModManifest> {
    let mut files = Vec::new();
    let mut paths: Vec<_> = walkdir::WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .collect();
    paths.sort();

    for path in paths {
        let Some(rel) = rel_path(dir, &path) else {
            continue;
        };
        if rel == SRF_FILE_NAME || rel.starts_with(".nimby/") {
            continue;
        }
        let length = fs::metadata(&path)?.len();
        let mut kind = infer_kind(&rel);
        let parts = match kind {
            FileKind::Pbo => match pbo::parts(&path) {
                Ok(parts) => parts,
                Err(_) => {
                    // A .pbo that is not a real PBO archive is hashed as one blob.
                    kind = FileKind::File;
                    single_part(&path, length)?
                }
            },
            FileKind::File => single_part(&path, length)?,
        };
        files.push(ModFile {
            path: rel,
            length,
            checksum: file_checksum(&parts),
            kind,
            parts,
        });
    }

    let checksum = mod_checksum(&files);
    Ok(ModManifest {
        name: name.to_string(),
        checksum,
        files,
        format: SrfFormat::V3,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CBA_WS_V3: &str = "\u{feff}{\"Name\":\"@cba_ws\",\"Checksum\":\"DD39F3C34318C8CB5950E0B785FBCB08\",\"Files\":[{\"Path\":\"keys/pallas_cba_ws.bikey\",\"Length\":295,\"Checksum\":\"9EACC8526973572A079036CDAD7B7D4F\",\"Type\":\"SwiftyFile\",\"Parts\":[{\"Path\":\"pallas_cba_ws.bikey_295\",\"Length\":295,\"Start\":0,\"Checksum\":\"6CA9404C72F39276317A8F358D8A50ED\"}]},{\"Path\":\"mod.cpp\",\"Length\":895,\"Checksum\":\"0A266973E65CDA6F39BC5C140AC937FC\",\"Type\":\"SwiftyFile\",\"Parts\":[{\"Path\":\"mod.cpp_895\",\"Length\":895,\"Start\":0,\"Checksum\":\"32DCD6F6EB27CF18E4917106356ADCED\"}]}]}";

    #[test]
    fn parses_v3_json_with_bom() {
        let manifest = ModManifest::parse(CBA_WS_V3.as_bytes()).unwrap();
        assert_eq!(manifest.format, SrfFormat::V3);
        assert_eq!(manifest.name, "@cba_ws");
        assert_eq!(manifest.files.len(), 2);
        assert_eq!(manifest.files[1].kind, FileKind::File);
        assert_eq!(manifest.files[1].parts[0].start, 0);
    }

    #[test]
    fn v3_round_trip_keeps_checksums() {
        let manifest = ModManifest::parse(CBA_WS_V3.as_bytes()).unwrap();
        let text = manifest.to_v3_json();
        assert!(text.starts_with('\u{feff}'));
        let again = ModManifest::parse(text.as_bytes()).unwrap();
        assert_eq!(manifest, again);
    }

    #[test]
    fn parses_legacy_text() {
        let legacy = "ADDON:@cba_ws:2:C990CED75EB2A4A5C6B6A0A3A0A7A4A5\n\
PBO:addons\\x.pbo:100:2:5670E8F32A56599E68CAD0ECD83B2380\n\
$$HEADER$$:0:40:BD24FBB68EF67070E883C54995F32543\n\
config.cpp:40:60:2CBA6F17C3C5AAD66E34C4EAA2977051\n\
FILE:mod.cpp:895:1:0A266973E65CDA6F39BC5C140AC937FC\n\
mod.cpp_895:0:895:32DCD6F6EB27CF18E4917106356ADCED\n";
        let manifest = ModManifest::parse(legacy.as_bytes()).unwrap();
        assert_eq!(manifest.format, SrfFormat::Legacy);
        assert_eq!(manifest.files.len(), 2);
        assert_eq!(manifest.files[0].path, "addons/x.pbo");
        assert_eq!(manifest.files[0].kind, FileKind::Pbo);
        assert_eq!(manifest.files[0].parts.len(), 2);
        assert_eq!(manifest.files[1].parts[0].length, 895);
    }

    #[test]
    fn legacy_round_trip() {
        let legacy =
            "ADDON:@x:1:00000000000000000000000000000000\nFILE:mod.cpp:1:1:AA\nmod.cpp_1:0:1:BB\n";
        let manifest = ModManifest::parse(legacy.as_bytes()).unwrap();
        let text = manifest.to_legacy_text();
        let again = ModManifest::parse(text.as_bytes()).unwrap();
        assert_eq!(manifest, again);
    }

    #[test]
    fn formatted_checksum_matches_format() {
        let mut manifest = ModManifest::parse(CBA_WS_V3.as_bytes()).unwrap();
        manifest.checksum = String::new();
        let v3 = manifest.clone().formatted(SrfFormat::V3);
        let legacy = manifest.formatted(SrfFormat::Legacy);
        assert_eq!(v3.checksum.len(), 32);
        assert_ne!(v3.checksum, legacy.checksum);

        let reparsed = ModManifest::parse(&legacy.to_bytes(SrfFormat::Legacy)).unwrap();
        assert_eq!(reparsed.format, SrfFormat::Legacy);
        assert_eq!(reparsed.checksum, legacy.checksum);
    }

    #[test]
    fn repo_json_is_case_insensitive_and_tolerant() {
        let text = r#"{
            "repoName": "X",
            "requiredMods": [{"modName": "@a", "checkSum": "AA", "enabled": true}],
            "optionalMods": [{"modname": "@b", "checksum": "BB"}],
            "servers": [{"Name": "s", "address": "1.2.3.4", "port": "2302", "battleEye": false}],
            "basePath": "addons",
            "iconImagePath": "icon.png"
        }"#;
        let repo = RepoManifest::parse(text.as_bytes()).unwrap();
        assert_eq!(repo.repo_name, "X");
        assert_eq!(repo.required_mods.len(), 1);
        assert_eq!(repo.optional_mods.len(), 1);
        assert!(repo.optional_mods[0].enabled);
        assert_eq!(repo.servers[0].address, "1.2.3.4");
        assert!(repo.extra.contains_key("basePath"));
    }
}
