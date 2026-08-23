use std::fs;
use std::path::PathBuf;

use crate::storage;

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

fn history_file() -> PathBuf {
    data_dir().join("math_history.txt")
}

pub fn load() -> Vec<String> {
    match fs::read_to_string(history_file()) {
        Ok(contents) => contents.lines().map(|l| l.to_string()).collect(),
        Err(_) => Vec::new(),
    }
}

pub fn append(entry: &str) -> std::io::Result<()> {
    if entry.contains(['\r', '\n']) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "history entries must be single-line",
        ));
    }

    let path = history_file();
    let _lock = storage::Lock::acquire(&path.with_extension("lock"))?;
    let mut entries = load();
    entries.push(entry.to_string());
    save_unlocked(&path, &entries)
}

pub fn save(entries: &[String]) -> std::io::Result<()> {
    let path = history_file();
    let _lock = storage::Lock::acquire(&path.with_extension("lock"))?;
    save_unlocked(&path, entries)
}

fn save_unlocked(path: &std::path::Path, entries: &[String]) -> std::io::Result<()> {
    let mut contents = String::new();
    for entry in entries {
        if entry.contains(['\r', '\n']) {
            continue;
        }
        contents.push_str(entry);
        contents.push('\n');
    }
    storage::atomic_write(path, contents.as_bytes())
}
