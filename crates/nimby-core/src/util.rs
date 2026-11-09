use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use crate::error::{Error, Result};

/// Strip a UTF-8 BOM from the start of a string, if present.
pub fn strip_bom(s: &str) -> &str {
    s.strip_prefix('\u{feff}').unwrap_or(s)
}

/// Convert a manifest path (either `\` or `/` separated) into a safe, canonical
/// relative path using `/` separators.
pub fn safe_rel_path(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(Error::UnsafePath(raw.to_string()));
    }
    if trimmed.chars().any(|c| c == '\0' || (c as u32) < 0x20) {
        return Err(Error::UnsafePath(raw.to_string()));
    }
    // Reject drive prefixes, UNC paths and rooted paths before splitting.
    let bytes = trimmed.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' {
        return Err(Error::UnsafePath(raw.to_string()));
    }
    if trimmed.starts_with('/') || trimmed.starts_with('\\') {
        return Err(Error::UnsafePath(raw.to_string()));
    }
    if trimmed.contains(':') {
        return Err(Error::UnsafePath(raw.to_string()));
    }

    let mut parts: Vec<&str> = Vec::new();
    for comp in trimmed.split(['/', '\\']) {
        if comp.is_empty() || comp == "." {
            continue;
        }
        if comp == ".." {
            return Err(Error::UnsafePath(raw.to_string()));
        }
        if comp.ends_with(' ') || comp.ends_with('.') {
            return Err(Error::UnsafePath(raw.to_string()));
        }
        if is_reserved_windows_name(comp) {
            return Err(Error::UnsafePath(raw.to_string()));
        }
        parts.push(comp);
    }
    if parts.is_empty() {
        return Err(Error::UnsafePath(raw.to_string()));
    }
    Ok(parts.join("/"))
}

fn is_reserved_windows_name(comp: &str) -> bool {
    let stem = comp.split('.').next().unwrap_or(comp).to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

/// Join a safe relative path onto a root directory.
pub fn join_rel(root: &Path, rel: &str) -> Result<PathBuf> {
    let safe = safe_rel_path(rel)?;
    let mut out = root.to_path_buf();
    for comp in safe.split('/') {
        out.push(comp);
    }
    // Defense in depth: the result must stay under root.
    if !starts_with_path(&out, root) {
        return Err(Error::UnsafePath(rel.to_string()));
    }
    Ok(out)
}

fn starts_with_path(path: &Path, root: &Path) -> bool {
    path.starts_with(root)
}

/// Path of `path` relative to `root`, using `/` separators.
pub fn rel_path(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let mut parts = Vec::new();
    for comp in rel.components() {
        match comp {
            Component::Normal(s) => parts.push(s.to_string_lossy().to_string()),
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(parts.join("/"))
}

/// Case-insensitive JSON object lookup.
pub fn get_ci<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    let obj = value.as_object()?;
    if let Some(v) = obj.get(key) {
        return Some(v);
    }
    obj.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v)
}

pub fn get_ci_string(value: &Value, key: &str) -> Option<String> {
    get_ci(value, key).and_then(|v| match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    })
}

pub fn get_ci_u64(value: &Value, key: &str) -> Option<u64> {
    get_ci(value, key).and_then(|v| match v {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    })
}

pub fn get_ci_bool(value: &Value, key: &str) -> Option<bool> {
    get_ci(value, key).and_then(|v| match v {
        Value::Bool(b) => Some(*b),
        Value::String(s) => match s.to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => Some(true),
            "false" | "0" | "no" => Some(false),
            _ => None,
        },
        Value::Number(n) => n.as_u64().map(|n| n != 0),
        _ => None,
    })
}

pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_normal_paths() {
        assert_eq!(safe_rel_path("addons/a.pbo").unwrap(), "addons/a.pbo");
        assert_eq!(
            safe_rel_path("addons\\sub\\a.pbo").unwrap(),
            "addons/sub/a.pbo"
        );
        assert_eq!(safe_rel_path("./mod.cpp").unwrap(), "mod.cpp");
    }

    #[test]
    fn rejects_escaping_paths() {
        for bad in [
            "../x.pbo",
            "addons/../../x.pbo",
            "C:/x.pbo",
            "/etc/passwd",
            "\\\\server\\share\\x",
            "addons/con.pbo",
            "addons/trailing.pbo. ",
        ] {
            assert!(safe_rel_path(bad).is_err(), "should reject {bad}");
        }
    }

    #[test]
    fn bom_is_stripped() {
        assert_eq!(strip_bom("\u{feff}{}"), "{}");
    }
}
