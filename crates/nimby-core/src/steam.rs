use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::Result;

pub const ARMA3_APP_ID: u32 = 107410;

// ---------------------------------------------------------------------------
// Minimal Valve KeyValues (VDF) parser
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum Vdf {
    Str(String),
    Map(BTreeMap<String, Vdf>),
}

impl Vdf {
    pub fn get(&self, key: &str) -> Option<&Vdf> {
        match self {
            Vdf::Map(map) => map.get(key),
            Vdf::Str(_) => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Vdf::Str(s) => Some(s),
            Vdf::Map(_) => None,
        }
    }

    pub fn as_map(&self) -> Option<&BTreeMap<String, Vdf>> {
        match self {
            Vdf::Map(m) => Some(m),
            Vdf::Str(_) => None,
        }
    }
}

pub fn parse_vdf(text: &str) -> Vdf {
    fn tokenize(text: &str) -> Vec<Token> {
        let mut tokens = Vec::new();
        let bytes: Vec<char> = text.chars().collect();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            if c.is_whitespace() {
                i += 1;
            } else if c == '/' && i + 1 < bytes.len() && bytes[i + 1] == '/' {
                while i < bytes.len() && bytes[i] != '\n' {
                    i += 1;
                }
            } else if c == '{' {
                tokens.push(Token::Open);
                i += 1;
            } else if c == '}' {
                tokens.push(Token::Close);
                i += 1;
            } else if c == '"' {
                i += 1;
                let mut value = String::new();
                while i < bytes.len() && bytes[i] != '"' {
                    if bytes[i] == '\\' && i + 1 < bytes.len() {
                        // VDF escapes backslashes inside quoted strings.
                        i += 1;
                        value.push(bytes[i]);
                    } else {
                        value.push(bytes[i]);
                    }
                    i += 1;
                }
                i += 1;
                tokens.push(Token::Str(value));
            } else {
                let mut value = String::new();
                while i < bytes.len()
                    && !bytes[i].is_whitespace()
                    && bytes[i] != '{'
                    && bytes[i] != '}'
                {
                    value.push(bytes[i]);
                    i += 1;
                }
                tokens.push(Token::Str(value));
            }
        }
        tokens
    }

    enum Token {
        Str(String),
        Open,
        Close,
    }

    fn parse_map(tokens: &[Token], pos: &mut usize) -> BTreeMap<String, Vdf> {
        let mut map = BTreeMap::new();
        while *pos < tokens.len() {
            match &tokens[*pos] {
                Token::Close => {
                    *pos += 1;
                    return map;
                }
                Token::Open => {
                    *pos += 1;
                    let _ = parse_map(tokens, pos);
                }
                Token::Str(key) => {
                    let key = key.clone();
                    *pos += 1;
                    match tokens.get(*pos) {
                        Some(Token::Str(value)) => {
                            map.insert(key, Vdf::Str(value.clone()));
                            *pos += 1;
                        }
                        Some(Token::Open) => {
                            *pos += 1;
                            let child = parse_map(tokens, pos);
                            map.insert(key, Vdf::Map(child));
                        }
                        _ => break,
                    }
                }
            }
        }
        map
    }

    let tokens = tokenize(text);
    let mut pos = 0;
    Vdf::Map(parse_map(&tokens, &mut pos))
}

// ---------------------------------------------------------------------------
// Steam discovery
// ---------------------------------------------------------------------------

pub fn steam_root() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};
        use winreg::RegKey;

        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok(key) = hkcu.open_subkey_with_flags(r"Software\Valve\Steam", KEY_READ) {
            if let Ok(path) = key.get_value::<String, _>("SteamPath") {
                let p = PathBuf::from(path.replace('/', "\\"));
                if p.is_dir() {
                    return Some(p);
                }
            }
        }
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        for sub in [r"SOFTWARE\WOW6432Node\Valve\Steam", r"SOFTWARE\Valve\Steam"] {
            if let Ok(key) = hklm.open_subkey_with_flags(sub, KEY_READ) {
                if let Ok(path) = key.get_value::<String, _>("InstallPath") {
                    let p = PathBuf::from(path.replace('/', "\\"));
                    if p.is_dir() {
                        return Some(p);
                    }
                }
            }
        }
        None
    }
    #[cfg(unix)]
    {
        let home = std::env::var_os("HOME")?;
        for rel in [".steam/steam", ".local/share/Steam"] {
            let p = PathBuf::from(&home).join(rel);
            if p.is_dir() {
                return Some(p);
            }
        }
        None
    }
}

/// All Steam library roots (the main install plus every library folder).
pub fn library_folders() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Some(root) = steam_root() else {
        return out;
    };
    out.push(root.clone());

    let vdf_path = root.join("steamapps").join("libraryfolders.vdf");
    if let Ok(text) = fs::read_to_string(&vdf_path) {
        let vdf = parse_vdf(&text);
        if let Some(map) = vdf.get("libraryfolders").and_then(|v| v.as_map()) {
            for (key, value) in map {
                if key.chars().all(|c| c.is_ascii_digit()) {
                    if let Some(path) = value.get("path").and_then(|v| v.as_str()) {
                        let p = PathBuf::from(path);
                        if p.is_dir() && !out.contains(&p) {
                            out.push(p);
                        }
                    }
                }
            }
        }
    }
    out
}

pub fn find_arma3() -> Option<PathBuf> {
    for lib in library_folders() {
        let p = lib.join("steamapps").join("common").join("Arma 3");
        if p.join("arma3_x64.exe").is_file() || p.join("arma3.exe").is_file() {
            return Some(p);
        }
    }
    None
}

pub fn workshop_content_dirs(app_id: u32) -> Vec<PathBuf> {
    library_folders()
        .into_iter()
        .map(|lib| {
            lib.join("steamapps")
                .join("workshop")
                .join("content")
                .join(app_id.to_string())
        })
        .filter(|p| p.is_dir())
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkshopItem {
    pub published_id: String,
    pub root: PathBuf,
    pub name: Option<String>,
    pub size: Option<u64>,
    pub time_updated: Option<u64>,
    pub manifest: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct MetaCpp {
    pub published_id: Option<String>,
    pub name: Option<String>,
    pub timestamp: Option<String>,
}

pub fn parse_meta_cpp(text: &str) -> MetaCpp {
    let mut meta = MetaCpp::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim().trim_end_matches(';').trim();
        let value = value.trim_matches('"').to_string();
        match key.as_str() {
            "publishedid" => meta.published_id = Some(value),
            "name" => meta.name = Some(value),
            "timestamp" => meta.timestamp = Some(value),
            _ => {}
        }
    }
    meta
}

/// Read `appworkshop_<app_id>.acf` for a library: published id -> item info.
pub fn acf_items(library: &Path, app_id: u32) -> BTreeMap<String, WorkshopItem> {
    let mut out = BTreeMap::new();
    let path = library
        .join("steamapps")
        .join("workshop")
        .join(format!("appworkshop_{app_id}.acf"));
    let Ok(text) = fs::read_to_string(&path) else {
        return out;
    };
    let vdf = parse_vdf(&text);
    let Some(items) = vdf
        .get("AppWorkshop")
        .and_then(|v| v.get("WorkshopItemsInstalled"))
        .and_then(|v| v.as_map())
    else {
        return out;
    };
    let content = library
        .join("steamapps")
        .join("workshop")
        .join("content")
        .join(app_id.to_string());
    for (id, value) in items {
        out.insert(
            id.clone(),
            WorkshopItem {
                published_id: id.clone(),
                root: content.join(id),
                name: None,
                size: value
                    .get("size")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse().ok()),
                time_updated: value
                    .get("timeupdated")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse().ok()),
                manifest: value
                    .get("manifest")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
            },
        );
    }
    out
}

/// Scan every local Arma 3 workshop item.
pub fn scan_workshop(app_id: u32) -> Vec<WorkshopItem> {
    let mut items: BTreeMap<String, WorkshopItem> = BTreeMap::new();
    for lib in library_folders() {
        items.extend(acf_items(&lib, app_id));
    }
    for dir in workshop_content_dirs(app_id) {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let id = entry.file_name().to_string_lossy().to_string();
            let root = entry.path();
            let meta = fs::read_to_string(root.join("meta.cpp"))
                .map(|t| parse_meta_cpp(&t))
                .unwrap_or_default();
            let item = items.entry(id.clone()).or_insert(WorkshopItem {
                published_id: id.clone(),
                root: root.clone(),
                name: None,
                size: None,
                time_updated: None,
                manifest: None,
            });
            item.root = root;
            if item.name.is_none() {
                item.name = meta.name;
            }
        }
    }
    items.into_values().collect()
}

/// Read the `publishedid` embedded in a repo mod's `meta.cpp`, if any.
pub fn repo_mod_published_id(mod_dir: &Path) -> Option<String> {
    let text = fs::read_to_string(mod_dir.join("meta.cpp")).ok()?;
    parse_meta_cpp(&text).published_id
}

pub fn read_workshop_meta(root: &Path) -> Result<MetaCpp> {
    let text = fs::read_to_string(root.join("meta.cpp"))?;
    Ok(parse_meta_cpp(&text))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACF: &str = r#""AppWorkshop"
{
    "appid"        "107410"
    "SizeOnDisk"    "100"
    "WorkshopItemsInstalled"
    {
        "450814997"
        {
            "size"      "12345"
            "timeupdated"   "1700000000"
            "manifest"  "-1"
            "ugchandle" "42"
        }
    }
}"#;

    #[test]
    fn parses_acf() {
        let vdf = parse_vdf(ACF);
        let items = vdf
            .get("AppWorkshop")
            .and_then(|v| v.get("WorkshopItemsInstalled"))
            .and_then(|v| v.as_map())
            .unwrap();
        let item = items.get("450814997").unwrap();
        assert_eq!(item.get("size").unwrap().as_str(), Some("12345"));
        assert_eq!(item.get("manifest").unwrap().as_str(), Some("-1"));
    }

    #[test]
    fn parses_meta_cpp() {
        let meta = parse_meta_cpp(
            "protocol = 1;\npublishedid = 3013893543;\nname = \"Western Sahara CBA Compatibility Patch\";\ntimestamp = 5250212105350556193;",
        );
        assert_eq!(meta.published_id.as_deref(), Some("3013893543"));
        assert_eq!(
            meta.name.as_deref(),
            Some("Western Sahara CBA Compatibility Patch")
        );
    }

    #[test]
    fn parses_library_folders() {
        let text = r#""libraryfolders"
{
    "0" { "path" "C:\\Program Files (x86)\\Steam" }
    "1" { "path" "E:\\SteamLibrary" }
}"#;
        let vdf = parse_vdf(text);
        let map = vdf.get("libraryfolders").and_then(|v| v.as_map()).unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(
            map.get("1").unwrap().get("path").unwrap().as_str(),
            Some("E:\\SteamLibrary")
        );
    }
}
