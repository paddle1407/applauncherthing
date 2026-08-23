use std::fs;
use std::path::PathBuf;

use crate::storage;

const MAX_ENTRIES: usize = 50;

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

fn recents_file() -> PathBuf {
    data_dir().join("recents.txt")
}

pub fn load() -> Vec<String> {
    match fs::read_to_string(recents_file()) {
        Ok(contents) => contents.lines().map(String::from).collect(),
        Err(_) => Vec::new(),
    }
}

pub fn record(name: &str) -> Vec<String> {
    let path = recents_file();
    let Ok(_lock) = storage::Lock::acquire(&path.with_extension("lock")) else {
        return load();
    };
    let mut list = load();

    list.retain(|n| n != name);
    list.insert(0, name.to_string());
    list.truncate(MAX_ENTRIES);

    let _ = save_unlocked(&path, &list);

    list
}

fn save_unlocked(path: &std::path::Path, list: &[String]) -> std::io::Result<()> {
    let mut contents = String::new();
    for name in list {
        if name.contains(['\r', '\n']) {
            continue;
        }
        contents.push_str(name);
        contents.push('\n');
    }
    storage::atomic_write(path, contents.as_bytes())
}
