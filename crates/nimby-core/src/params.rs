//! Arma 3 launch parameters: a small typed model so the UI can render proper
//! controls (checkbox, text box, dropdown) instead of a raw command line.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamKind {
    /// No value, for example `-noSplash`.
    Flag,
    /// Free text, for example `-name=`.
    Text,
    /// Integer, for example `-maxMem=`.
    Number,
    /// Filesystem path, for example `-profiles=`.
    Path,
    /// One of a fixed set, for example `-exThreads=`.
    Choice,
}

/// Description of a parameter Arma 3 understands.
#[derive(Clone, Copy, Debug)]
pub struct ParamSpec {
    pub key: &'static str,
    pub label: &'static str,
    pub kind: ParamKind,
    /// Suggested values for `ParamKind::Choice`.
    pub choices: &'static [&'static str],
    /// Always wrap the value in double quotes (profile names with spaces).
    pub always_quote: bool,
    pub description: &'static str,
    /// Prefix the value with `-` and use it as a bare flag when empty.
    pub managed_by_nimby: bool,
}

impl ParamSpec {
    pub fn needs_value(&self) -> bool {
        self.kind != ParamKind::Flag
    }
}

const EX_THREADS: &[&str] = &["0", "1", "3", "5", "7", "15"];
const MALLOC: &[&str] = &[
    "tbb4malloc_bi",
    "tbbmalloc",
    "jemalloc_bi",
    "jemalloc",
    "system_malloc",
];

/// Parameters worth exposing in the UI. Anything else can still be typed as a
/// free-form parameter.
pub const KNOWN_PARAMS: &[ParamSpec] = &[
    ParamSpec {
        key: "-name",
        label: "Profile name",
        kind: ParamKind::Text,
        choices: &[],
        always_quote: true,
        description:
            "Arma profile folder name. Use a different one per repo to keep keybinds separate.",
        managed_by_nimby: false,
    },
    ParamSpec {
        key: "-connect",
        label: "Server address",
        kind: ParamKind::Text,
        choices: &[],
        always_quote: false,
        description: "Join this server directly. Usually set from the repo server list.",
        managed_by_nimby: true,
    },
    ParamSpec {
        key: "-port",
        label: "Server port",
        kind: ParamKind::Number,
        choices: &[],
        always_quote: false,
        description: "Game port of the server (default 2302).",
        managed_by_nimby: true,
    },
    ParamSpec {
        key: "-password",
        label: "Server password",
        kind: ParamKind::Text,
        choices: &[],
        always_quote: false,
        description: "Password sent to the server. Redacted in logs.",
        managed_by_nimby: true,
    },
    ParamSpec {
        key: "-mod",
        label: "Mod list",
        kind: ParamKind::Path,
        choices: &[],
        always_quote: false,
        description: "Built automatically from the installed mods.",
        managed_by_nimby: true,
    },
    ParamSpec {
        key: "-serverMod",
        label: "Server-side mods",
        kind: ParamKind::Path,
        choices: &[],
        always_quote: false,
        description: "Additional server-side-only mods.",
        managed_by_nimby: false,
    },
    ParamSpec {
        key: "-world",
        label: "World",
        kind: ParamKind::Text,
        choices: &[],
        always_quote: false,
        description: "Load straight into a terrain, for example `Altis`.",
        managed_by_nimby: false,
    },
    ParamSpec {
        key: "-mission",
        label: "Mission",
        kind: ParamKind::Text,
        choices: &[],
        always_quote: false,
        description: "Load a mission on start.",
        managed_by_nimby: false,
    },
    ParamSpec {
        key: "-malloc",
        label: "Memory allocator",
        kind: ParamKind::Choice,
        choices: MALLOC,
        always_quote: false,
        description: "Allocator implementation; big modsets often benefit from tbb4malloc_bi.",
        managed_by_nimby: false,
    },
    ParamSpec {
        key: "-maxMem",
        label: "Max memory (MB)",
        kind: ParamKind::Number,
        choices: &[],
        always_quote: false,
        description: "Memory limit in megabytes (2047 default, 0 = unlimited on x64).",
        managed_by_nimby: false,
    },
    ParamSpec {
        key: "-maxVRAM",
        label: "Max VRAM (MB)",
        kind: ParamKind::Number,
        choices: &[],
        always_quote: false,
        description: "VRAM limit in megabytes.",
        managed_by_nimby: false,
    },
    ParamSpec {
        key: "-cpuCount",
        label: "CPU cores",
        kind: ParamKind::Number,
        choices: &[],
        always_quote: false,
        description: "Number of cores Arma is allowed to use.",
        managed_by_nimby: false,
    },
    ParamSpec {
        key: "-exThreads",
        label: "Extra threads",
        kind: ParamKind::Choice,
        choices: EX_THREADS,
        always_quote: false,
        description: "Threads for texture/geometry loading (0, 1, 3, 5, 7, 15).",
        managed_by_nimby: false,
    },
    ParamSpec {
        key: "-limitFPS",
        label: "FPS limit",
        kind: ParamKind::Number,
        choices: &[],
        always_quote: false,
        description: "Cap frames per second.",
        managed_by_nimby: false,
    },
    ParamSpec {
        key: "-profiles",
        label: "Profiles folder",
        kind: ParamKind::Path,
        choices: &[],
        always_quote: true,
        description: "Where Arma stores profiles. Useful to isolate repos.",
        managed_by_nimby: false,
    },
    ParamSpec {
        key: "-showScriptErrors",
        label: "Show script errors",
        kind: ParamKind::Flag,
        choices: &[],
        always_quote: false,
        description: "Display SQF errors on screen.",
        managed_by_nimby: false,
    },
    ParamSpec {
        key: "-filePatching",
        label: "File patching",
        kind: ParamKind::Flag,
        choices: &[],
        always_quote: false,
        description: "Allow loose-file overrides from the mod folders.",
        managed_by_nimby: false,
    },
    ParamSpec {
        key: "-noSplash",
        label: "No splash screens",
        kind: ParamKind::Flag,
        choices: &[],
        always_quote: false,
        description: "Skip publisher splash screens.",
        managed_by_nimby: false,
    },
    ParamSpec {
        key: "-skipIntro",
        label: "Skip intro",
        kind: ParamKind::Flag,
        choices: &[],
        always_quote: false,
        description: "Skip the main-menu intro cinematic.",
        managed_by_nimby: false,
    },
    ParamSpec {
        key: "-noPause",
        label: "No pause on focus loss",
        kind: ParamKind::Flag,
        choices: &[],
        always_quote: false,
        description: "Keep the game running when the window loses focus.",
        managed_by_nimby: false,
    },
    ParamSpec {
        key: "-window",
        label: "Windowed",
        kind: ParamKind::Flag,
        choices: &[],
        always_quote: false,
        description: "Start in windowed mode.",
        managed_by_nimby: false,
    },
    ParamSpec {
        key: "-enableHT",
        label: "Enable hyper-threading",
        kind: ParamKind::Flag,
        choices: &[],
        always_quote: false,
        description: "Let Arma use hyper-threaded cores.",
        managed_by_nimby: false,
    },
    ParamSpec {
        key: "-hugePages",
        label: "Huge pages",
        kind: ParamKind::Flag,
        choices: &[],
        always_quote: false,
        description: "Use large memory pages (requires OS support).",
        managed_by_nimby: false,
    },
    ParamSpec {
        key: "-noLogs",
        label: "No log files",
        kind: ParamKind::Flag,
        choices: &[],
        always_quote: false,
        description: "Disable RPT log writing.",
        managed_by_nimby: false,
    },
];

pub fn find_spec(key: &str) -> Option<&'static ParamSpec> {
    KNOWN_PARAMS
        .iter()
        .find(|spec| spec.key.eq_ignore_ascii_case(key.trim()))
}

/// A single configured launch parameter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchParam {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

impl LaunchParam {
    pub fn flag(key: &str) -> Self {
        Self {
            key: key.trim().to_string(),
            value: None,
            enabled: true,
        }
    }

    pub fn with_value(key: &str, value: impl Into<String>) -> Self {
        Self {
            key: key.trim().to_string(),
            value: Some(value.into()),
            enabled: true,
        }
    }

    /// Parse `-name=Fred`, `-name="Fred Flintstone"` or `-noSplash`.
    pub fn parse(input: &str) -> Self {
        let trimmed = input.trim();
        let (key, raw_value) = match trimmed.split_once('=') {
            Some((key, value)) => (key.trim(), Some(value.trim())),
            None => (trimmed, None),
        };
        let value = raw_value.and_then(|value| {
            let value = value.trim();
            let value = value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .unwrap_or(value);
            if value.is_empty() {
                None
            } else {
                Some(value.to_string())
            }
        });
        Self {
            key: key.to_string(),
            value,
            enabled: true,
        }
    }

    /// Render as an Arma command line argument.
    pub fn render(&self) -> String {
        let always_quote = find_spec(&self.key)
            .map(|spec| spec.always_quote)
            .unwrap_or(false);
        match &self.value {
            None => self.key.clone(),
            Some(value) if value.is_empty() => self.key.clone(),
            Some(value) => {
                let needs_quote = always_quote || value.contains(char::is_whitespace);
                if needs_quote {
                    format!("{}=\"{}\"", self.key, value.replace('"', ""))
                } else {
                    format!("{}={}", self.key, value)
                }
            }
        }
    }

    /// `-name="Fred"` style display text.
    pub fn display(&self) -> String {
        self.render()
    }

    /// Value shown in a `Key = value` row, empty for flags.
    pub fn value_text(&self) -> String {
        self.value.clone().unwrap_or_default()
    }

    pub fn is_valid(&self) -> bool {
        if self.key.trim().is_empty() || !self.key.starts_with('-') {
            return false;
        }
        match find_spec(&self.key) {
            Some(spec) if spec.needs_value() => self.value.is_some(),
            _ => true,
        }
    }
}

/// Render every enabled parameter in order.
pub fn render_all(params: &[LaunchParam]) -> Vec<String> {
    params
        .iter()
        .filter(|p| p.enabled && p.is_valid())
        .map(|p| p.render())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flag_and_valued_forms() {
        let flag = LaunchParam::parse("-noSplash");
        assert_eq!(flag.key, "-noSplash");
        assert!(flag.value.is_none());
        assert_eq!(flag.render(), "-noSplash");

        let value = LaunchParam::parse("-maxMem=8192");
        assert_eq!(value.key, "-maxMem");
        assert_eq!(value.value.as_deref(), Some("8192"));
        assert_eq!(value.render(), "-maxMem=8192");

        let quoted = LaunchParam::parse("-name=\"Fred Flintstone\"");
        assert_eq!(quoted.value.as_deref(), Some("Fred Flintstone"));
        assert_eq!(quoted.render(), "-name=\"Fred Flintstone\"");
    }

    #[test]
    fn profile_name_is_always_quoted() {
        let name = LaunchParam::with_value("-name", "Fred");
        assert_eq!(name.render(), "-name=\"Fred\"");
    }

    #[test]
    fn values_with_spaces_are_quoted() {
        let param = LaunchParam::with_value("-profiles", "C:\\My Games\\Arma");
        assert_eq!(param.render(), "-profiles=\"C:\\My Games\\Arma\"");
        let plain = LaunchParam::with_value("-world", "Altis");
        assert_eq!(plain.render(), "-world=Altis");
    }

    #[test]
    fn render_all_skips_disabled_and_invalid() {
        let mut disabled = LaunchParam::with_value("-world", "Altis");
        disabled.enabled = false;
        let params = vec![
            LaunchParam::flag("-noSplash"),
            disabled,
            LaunchParam::with_value("-name", "Fred"),
            LaunchParam {
                key: "not-a-flag".into(),
                value: None,
                enabled: true,
            },
        ];
        assert_eq!(
            render_all(&params),
            vec!["-noSplash".to_string(), "-name=\"Fred\"".to_string()]
        );
    }

    #[test]
    fn known_specs_are_well_formed() {
        for spec in KNOWN_PARAMS {
            assert!(spec.key.starts_with('-'), "{}", spec.key);
            if spec.kind == ParamKind::Choice {
                assert!(!spec.choices.is_empty(), "{} has no choices", spec.key);
            }
            assert!(find_spec(spec.key).is_some());
        }
    }
}
