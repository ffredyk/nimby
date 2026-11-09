use std::path::{Path, PathBuf};
use std::process::Child;

use crate::error::{Error, Result};

/// Windows command line limit for CreateProcess.
pub const MAX_COMMAND_LINE: usize = 8191;

#[derive(Clone, Debug, Default)]
pub struct LaunchOptions {
    pub arma_dir: PathBuf,
    pub mod_paths: Vec<PathBuf>,
    pub connect: Option<String>,
    pub port: Option<u16>,
    pub password: Option<String>,
    pub battleye: bool,
    pub extra_params: Vec<String>,
}

pub fn resolve_executable(arma_dir: &Path, battleye: bool) -> Result<PathBuf> {
    let candidates: &[&str] = if battleye {
        &["arma3battleye.exe", "arma3_x64.exe", "arma3.exe"]
    } else {
        &["arma3_x64.exe", "arma3.exe"]
    };
    for name in candidates {
        let path = arma_dir.join(name);
        if path.is_file() {
            return Ok(path);
        }
    }
    Err(Error::msg(format!(
        "no Arma 3 executable found in {}",
        arma_dir.display()
    )))
}

/// Split a parameter string, honouring double quotes.
pub fn split_params(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for ch in input.chars() {
        match ch {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

pub fn build_args(opts: &LaunchOptions) -> Result<Vec<String>> {
    let mut args = Vec::new();

    if !opts.mod_paths.is_empty() {
        let mut mods = Vec::new();
        for path in &opts.mod_paths {
            let text = path.display().to_string();
            if text.contains(';') {
                return Err(Error::msg(format!(
                    "mod path contains ';' which Arma cannot parse: {text}"
                )));
            }
            mods.push(text);
        }
        args.push(format!("-mod={}", mods.join(";")));
    }
    if let Some(host) = &opts.connect {
        args.push(format!("-connect={host}"));
    }
    if let Some(port) = opts.port {
        args.push(format!("-port={port}"));
    }
    if let Some(password) = &opts.password {
        if !password.is_empty() {
            args.push(format!("-password={password}"));
        }
    }
    args.extend(opts.extra_params.iter().cloned());
    Ok(args)
}

/// Replace the value of `-password=` so logs and dry runs never leak it.
pub fn redact_args(args: &[String]) -> Vec<String> {
    args.iter()
        .map(|arg| {
            if let Some(rest) = arg.strip_prefix("-password=") {
                if rest.is_empty() {
                    arg.clone()
                } else {
                    "-password=***".to_string()
                }
            } else {
                arg.clone()
            }
        })
        .collect()
}

pub fn command_line(opts: &LaunchOptions, redact: bool) -> Result<String> {
    let exe = resolve_executable(&opts.arma_dir, opts.battleye)?;
    let mut args = build_args(opts)?;
    if redact {
        args = redact_args(&args);
    }
    let mut line = format!("\"{}\"", exe.display());
    for arg in &args {
        if arg.contains(' ') {
            line.push_str(&format!(" \"{arg}\""));
        } else {
            line.push(' ');
            line.push_str(arg);
        }
    }

    // Windows caps the whole command line; Arma fails to start with a cryptic
    // error if it is exceeded, so fail early with something actionable.
    if line.chars().count() > MAX_COMMAND_LINE {
        let mods = opts.mod_paths.len();
        return Err(Error::msg(format!(
            "command line would be {} characters (limit {MAX_COMMAND_LINE}) with {mods} mods; \
             use a shorter workspace path, fewer mods, or a directory-linked workspace",
            line.chars().count()
        )));
    }
    Ok(line)
}

pub fn spawn(opts: &LaunchOptions) -> Result<Child> {
    let exe = resolve_executable(&opts.arma_dir, opts.battleye)?;
    let args = build_args(opts)?;
    let child = std::process::Command::new(exe).args(args).spawn()?;
    Ok(child)
}

/// Best-effort check whether Arma is running (never launch over a live game).
pub fn running_processes() -> Vec<String> {
    let mut out = Vec::new();

    #[cfg(windows)]
    {
        let output = std::process::Command::new("tasklist")
            .args(["/FO", "CSV", "/NH"])
            .output();
        if let Ok(output) = output {
            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines() {
                if let Some(name) = line.split(',').next() {
                    let name = name.trim_matches('"').to_ascii_lowercase();
                    if name.starts_with("arma3")
                        || name == "battleye.exe"
                        || name == "beservice.exe"
                    {
                        out.push(name);
                    }
                }
            }
        }
    }
    #[cfg(unix)]
    {
        if let Ok(entries) = std::fs::read_dir("/proc") {
            for entry in entries.flatten() {
                let comm = entry.path().join("comm");
                if let Ok(name) = std::fs::read_to_string(comm) {
                    let name = name.trim();
                    if name.starts_with("arma3") {
                        out.push(name.to_string());
                    }
                }
            }
        }
    }

    out
}

pub fn is_arma_running() -> bool {
    !running_processes().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_mod_argument() {
        let opts = LaunchOptions {
            arma_dir: PathBuf::from("C:/Arma 3"),
            mod_paths: vec![
                PathBuf::from(r"C:\Nimby\@cba"),
                PathBuf::from(r"C:\Nimby\@ace"),
            ],
            connect: Some("127.0.0.1".into()),
            port: Some(2302),
            password: Some("pw".into()),
            battleye: false,
            extra_params: vec!["-noSplash".into()],
        };
        let args = build_args(&opts).unwrap();
        assert_eq!(args[0], r"-mod=C:\Nimby\@cba;C:\Nimby\@ace");
        assert!(args.contains(&"-connect=127.0.0.1".to_string()));
        assert!(args.contains(&"-port=2302".to_string()));
        assert!(args.contains(&"-password=pw".to_string()));
    }

    #[test]
    fn splits_parameters_with_quotes() {
        assert_eq!(
            split_params(r#"-skipIntro -name="My Name" -x"#),
            vec!["-skipIntro", "-name=My Name", "-x"]
        );
    }

    #[test]
    fn redacts_password() {
        let args = vec![
            "-mod=C:\\a".to_string(),
            "-password=hunter2".to_string(),
            "-skipIntro".to_string(),
        ];
        let redacted = redact_args(&args);
        assert_eq!(redacted[1], "-password=***");
        assert_eq!(redacted[0], "-mod=C:\\a");
        assert_eq!(redacted[2], "-skipIntro");
    }

    #[test]
    fn rejects_command_line_over_the_windows_limit() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("arma3_x64.exe"), b"").unwrap();
        let opts = LaunchOptions {
            arma_dir: dir.path().to_path_buf(),
            mod_paths: (0..400)
                .map(|index| dir.path().join(format!("@mod{index:03}")))
                .collect(),
            ..Default::default()
        };
        let err = command_line(&opts, false).unwrap_err().to_string();
        assert!(err.contains("limit 8191"), "got: {err}");
    }

    #[test]
    fn rejects_semicolon_in_mod_path() {
        let opts = LaunchOptions {
            arma_dir: PathBuf::from("C:/Arma 3"),
            mod_paths: vec![PathBuf::from(r"C:\a;b")],
            ..Default::default()
        };
        assert!(build_args(&opts).is_err());
    }
}
