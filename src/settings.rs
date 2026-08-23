use std::fs;
use std::path::PathBuf;

use crate::storage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayMode {
    None,
    Apps,
    Recent,
}

impl DisplayMode {
    pub fn as_str(self) -> &'static str {
        match self {
            DisplayMode::None => "none",
            DisplayMode::Apps => "apps",
            DisplayMode::Recent => "recent",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "none" => Some(DisplayMode::None),
            "apps" => Some(DisplayMode::Apps),
            "recent" => Some(DisplayMode::Recent),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub background: bool,
    pub footer: bool,
    pub timers: bool,
    pub display: DisplayMode,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            background: false,
            footer: true,
            timers: true,
            display: DisplayMode::Recent,
        }
    }
}

fn data_dir() -> PathBuf {
    let base = match std::env::var("XDG_DATA_HOME") {
        Ok(data_home) if !data_home.is_empty() => PathBuf::from(data_home),
        _ => {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".local/share")
        }
    };
    base.join("mylauncher")
}

fn settings_file() -> PathBuf {
    data_dir().join("settings.txt")
}

pub fn load() -> Settings {
    let mut settings = Settings::default();

    if let Ok(contents) = fs::read_to_string(settings_file()) {
        for line in contents.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };

            match (key.trim(), value.trim()) {
                ("background", "true") => settings.background = true,
                ("background", "false") => settings.background = false,
                ("footer", "true") => settings.footer = true,
                ("footer", "false") => settings.footer = false,
                ("timers", "true") => settings.timers = true,
                ("timers", "false") => settings.timers = false,
                ("display", v) => {
                    if let Some(mode) = DisplayMode::from_str(v) {
                        settings.display = mode;
                    }
                }
                _ => {}
            }
        }
    }

    settings
}

pub fn save(settings: &Settings) -> std::io::Result<()> {
    let path = settings_file();
    let _lock = storage::Lock::acquire(&path.with_extension("lock"))?;
    let contents = format!(
        "background={}\nfooter={}\ntimers={}\ndisplay={}\n",
        settings.background,
        settings.footer,
        settings.timers,
        settings.display.as_str()
    );
    storage::atomic_write(&path, contents.as_bytes())
}
