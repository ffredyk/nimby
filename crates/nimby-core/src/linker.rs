use std::fs;
use std::path::Path;

use crate::error::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkKind {
    Existing,
    Junction,
    Symlink,
    Hardlink,
    Copy,
}

impl LinkKind {
    pub fn is_link(&self) -> bool {
        matches!(
            self,
            LinkKind::Junction | LinkKind::Symlink | LinkKind::Hardlink
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkMode {
    /// Pick the cheapest safe option per mod.
    Auto,
    /// Never link, always copy bytes.
    Copy,
    /// Link the whole mod directory when it matches exactly.
    DirLink,
    /// Link individual files, download the rest.
    Overlay,
    /// Do not materialize; launch straight from the Workshop folder.
    Passthrough,
}

impl LinkMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "auto" => Some(LinkMode::Auto),
            "copy" => Some(LinkMode::Copy),
            "dir-link" | "dirlink" | "dir" => Some(LinkMode::DirLink),
            "overlay" => Some(LinkMode::Overlay),
            "passthrough" => Some(LinkMode::Passthrough),
            _ => None,
        }
    }
}

/// True when the path itself is a reparse point (symlink or junction).
pub fn is_link(path: &Path) -> bool {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(unix)]
    {
        false
    }
}

pub fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

/// Create a directory link: junction first on Windows (no admin needed),
/// then symlink.
pub fn link_dir(link: &Path, target: &Path) -> Result<LinkKind> {
    if exists(link) {
        return Ok(LinkKind::Existing);
    }
    if let Some(parent) = link.parent() {
        fs::create_dir_all(parent)?;
    }

    #[cfg(windows)]
    {
        match win::create_junction(link, target) {
            Ok(()) => Ok(LinkKind::Junction),
            Err(junction_err) => match std::os::windows::fs::symlink_dir(target, link) {
                Ok(()) => Ok(LinkKind::Symlink),
                Err(symlink_err) => Err(Error::msg(format!(
                    "cannot link {} -> {}: junction: {junction_err}; symlink: {symlink_err}",
                    link.display(),
                    target.display()
                ))),
            },
        }
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)?;
        Ok(LinkKind::Symlink)
    }
}

/// Create a file link: hardlink first (no admin, same volume), then symlink.
pub fn link_file(link: &Path, target: &Path) -> Result<LinkKind> {
    if exists(link) {
        return Ok(LinkKind::Existing);
    }
    if let Some(parent) = link.parent() {
        fs::create_dir_all(parent)?;
    }
    if fs::hard_link(target, link).is_ok() {
        return Ok(LinkKind::Hardlink);
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_file(target, link)?;
        Ok(LinkKind::Symlink)
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)?;
        Ok(LinkKind::Symlink)
    }
}

/// Remove a file, link or directory tree.
pub fn remove_path(path: &Path) -> Result<()> {
    if !exists(path) {
        return Ok(());
    }
    let meta = fs::symlink_metadata(path)?;
    // Junctions and symlinks are removed as links; only a real directory is
    // deleted recursively.
    if meta.is_dir() && !is_link(path) {
        fs::remove_dir_all(path)?;
        return Ok(());
    }
    // A link may be reported as a file or a directory depending on the kind,
    // so try both removals before giving up.
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(file_err) => fs::remove_dir(path).map_err(|dir_err| {
            Error::msg(format!(
                "cannot remove {}: {file_err}; {dir_err}",
                path.display()
            ))
        }),
    }
}

/// Copy a file, creating parent directories and preserving nothing else.
pub fn copy_file(from: &Path, to: &Path) -> Result<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(from, to)?;
    Ok(())
}

#[cfg(windows)]
mod win {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows_sys::Win32::Foundation::{
        CloseHandle, GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::IO::DeviceIoControl;

    const FSCTL_SET_REPARSE_POINT: u32 = 0x0009_00A4;
    const IO_REPARSE_TAG_MOUNT_POINT: u32 = 0xA000_0003;

    fn wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(std::iter::once(0)).collect()
    }

    pub fn create_junction(link: &Path, target: &Path) -> std::io::Result<()> {
        std::fs::create_dir(link)?;

        let substitute = format!("\\??\\{}", target.display());
        let print_name = target.display().to_string();
        let substitute: Vec<u16> = substitute.encode_utf16().collect();
        let print_name: Vec<u16> = print_name.encode_utf16().collect();

        let substitute_bytes = substitute.len() * 2;
        let print_bytes = print_name.len() * 2;
        let path_buffer_len = substitute_bytes + 2 + print_bytes + 2;
        let reparse_data_len = 8 + path_buffer_len;

        // REPARSE_DATA_BUFFER: tag(4) + data length(2) + reserved(2) then data.
        let mut buffer = vec![0u8; 8 + reparse_data_len];
        buffer[0..4].copy_from_slice(&IO_REPARSE_TAG_MOUNT_POINT.to_le_bytes());
        buffer[4..6].copy_from_slice(&(reparse_data_len as u16).to_le_bytes());
        buffer[8..10].copy_from_slice(&0u16.to_le_bytes());
        buffer[10..12].copy_from_slice(&(substitute_bytes as u16).to_le_bytes());
        buffer[12..14].copy_from_slice(&((substitute_bytes + 2) as u16).to_le_bytes());
        buffer[14..16].copy_from_slice(&(print_bytes as u16).to_le_bytes());

        let mut offset = 16;
        for unit in &substitute {
            buffer[offset..offset + 2].copy_from_slice(&unit.to_le_bytes());
            offset += 2;
        }
        offset += 2;
        for unit in &print_name {
            buffer[offset..offset + 2].copy_from_slice(&unit.to_le_bytes());
            offset += 2;
        }

        let link_wide = wide(link.as_os_str());

        let mut last_err = None;
        let mut handle = INVALID_HANDLE_VALUE;
        // Some systems reject GENERIC_WRITE alone on a directory handle while
        // accepting the read+write combination, so try both.
        for access in [GENERIC_READ | GENERIC_WRITE, GENERIC_WRITE] {
            let candidate = unsafe {
                CreateFileW(
                    link_wide.as_ptr(),
                    access,
                    FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                    std::ptr::null_mut(),
                )
            };
            if candidate != INVALID_HANDLE_VALUE {
                handle = candidate;
                last_err = None;
                break;
            }
            last_err = Some(std::io::Error::last_os_error());
        }
        let handle = handle;
        if handle == INVALID_HANDLE_VALUE {
            let e = last_err.unwrap_or_else(std::io::Error::last_os_error);
            let _ = std::fs::remove_dir(link);
            return Err(std::io::Error::new(e.kind(), format!("open handle: {e}")));
        }

        let mut returned = 0u32;
        let ok = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_SET_REPARSE_POINT,
                buffer.as_ptr() as *const _,
                buffer.len() as u32,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        unsafe { CloseHandle(handle) };
        if ok == 0 {
            let e = std::io::Error::last_os_error();
            let _ = std::fs::remove_dir(link);
            return Err(std::io::Error::new(
                e.kind(),
                format!("set reparse point: {e}"),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_dir_link_and_remove() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("a.txt"), b"hello").unwrap();

        let link = dir.path().join("link");
        let kind = link_dir(&link, &target).unwrap();
        assert!(matches!(kind, LinkKind::Junction | LinkKind::Symlink));
        assert!(is_link(&link));
        assert_eq!(fs::read(link.join("a.txt")).unwrap(), b"hello");

        remove_path(&link).unwrap();
        assert!(!exists(&link));
        assert!(target.is_dir(), "target must survive link removal");
    }

    #[test]
    fn file_link_points_at_same_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("t.bin");
        fs::write(&target, b"abc").unwrap();
        let link = dir.path().join("nested").join("l.bin");
        let kind = link_file(&link, &target).unwrap();
        assert!(kind.is_link());
        assert_eq!(fs::read(&link).unwrap(), b"abc");
    }
}
