use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone)]
pub struct AppAction {
    #[allow(dead_code)]
    pub id: String,
    pub name: String,
    pub exec: Vec<String>,
    pub icon: Option<String>,
}

#[derive(Debug, Clone)]
pub struct App {
    pub name: String,
    pub exec: Vec<String>,
    pub icon: Option<String>,
    pub actions: Vec<AppAction>,
    pub terminal: bool,
    pub path: Option<PathBuf>,
    pub dbus_activatable: bool,
    pub desktop_file: PathBuf,
}

fn desktop_file_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    match env::var("XDG_DATA_HOME") {
        Ok(data_home) if !data_home.is_empty() => {
            dirs.push(PathBuf::from(data_home).join("applications"));
        }
        _ => {
            if let Ok(home) = env::var("HOME") {
                dirs.push(PathBuf::from(home).join(".local/share/applications"));
            }
        }
    }

    match env::var("XDG_DATA_DIRS") {
        Ok(data_dirs) if !data_dirs.is_empty() => {
            for dir in data_dirs.split(':').filter(|s| !s.is_empty()) {
                dirs.push(PathBuf::from(dir).join("applications"));
            }
        }
        _ => {
            dirs.push(PathBuf::from("/usr/local/share/applications"));
            dirs.push(PathBuf::from("/usr/share/applications"));
        }
    }

    dirs
}

fn parse_bool(fields: &HashMap<String, String>, key: &str) -> bool {
    fields
        .get(key)
        .is_some_and(|value| value.eq_ignore_ascii_case("true"))
}

fn current_desktops() -> Vec<String> {
    let mut desktops = Vec::new();

    if let Ok(value) = env::var("XDG_CURRENT_DESKTOP") {
        desktops.extend(
            value
                .split(':')
                .filter(|s| !s.is_empty())
                .map(str::to_string),
        );
    }

    if let Ok(value) = env::var("DESKTOP_SESSION")
        && !value.is_empty()
        && !desktops.iter().any(|desktop| desktop == &value)
    {
        desktops.push(value);
    }

    desktops
}

fn desktop_is_visible(fields: &HashMap<String, String>) -> bool {
    let desktops = current_desktops();

    if let Some(only_show_in) = fields.get("OnlyShowIn") {
        let allowed: Vec<&str> = only_show_in.split(';').filter(|s| !s.is_empty()).collect();
        if !allowed.is_empty()
            && !desktops.iter().any(|desktop| {
                allowed
                    .iter()
                    .any(|allowed| allowed.eq_ignore_ascii_case(desktop))
            })
        {
            return false;
        }
    }

    if let Some(not_show_in) = fields.get("NotShowIn") {
        let blocked: Vec<&str> = not_show_in.split(';').filter(|s| !s.is_empty()).collect();
        if desktops.iter().any(|desktop| {
            blocked
                .iter()
                .any(|blocked| blocked.eq_ignore_ascii_case(desktop))
        }) {
            return false;
        }
    }

    true
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }

    #[cfg(not(unix))]
    {
        metadata.is_file()
    }
}

fn command_available(command: &str) -> bool {
    let path = Path::new(command);

    if path.components().count() > 1 {
        return is_executable(path);
    }

    env::var_os("PATH").is_some_and(|path_var| {
        env::split_paths(&path_var).any(|dir| is_executable(&dir.join(command)))
    })
}

fn parse_exec(input: &str) -> Option<Vec<String>> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut token_started = false;

    for ch in input.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            token_started = true;
        } else if ch == '\\' {
            escaped = true;
            token_started = true;
        } else if (ch == '"' || ch == '\'') && (quote.is_none() || quote == Some(ch)) {
            if quote.is_some() {
                quote = None;
            } else {
                quote = Some(ch);
            }
            token_started = true;
        } else if ch.is_whitespace() && quote.is_none() {
            if token_started {
                args.push(std::mem::take(&mut current));
                token_started = false;
            }
        } else {
            current.push(ch);
            token_started = true;
        }
    }

    if escaped || quote.is_some() {
        return None;
    }

    if token_started {
        args.push(current);
    }

    if args.is_empty() || args.iter().any(|arg| !valid_field_codes(arg)) {
        None
    } else {
        Some(args)
    }
}

fn valid_field_codes(arg: &str) -> bool {
    let mut chars = arg.chars();

    while let Some(ch) = chars.next() {
        if ch != '%' {
            continue;
        }

        match chars.next() {
            Some('%') => {}
            Some('f' | 'F' | 'u' | 'U' | 'i' | 'c' | 'k')
                if arg.len() == 2 && arg.starts_with('%') => {}
            _ => return false,
        }
    }

    true
}

fn try_exec_available(value: &str) -> bool {
    let Some(command) = parse_exec(value).and_then(|args| args.into_iter().next()) else {
        return false;
    };

    command_available(&command)
}

fn parse_desktop_file(path: &Path) -> Option<App> {
    let content = fs::read_to_string(path).ok()?;
    let mut current_section = String::new();
    let mut main_fields: HashMap<String, String> = HashMap::new();
    let mut action_fields: HashMap<String, HashMap<String, String>> = HashMap::new();

    for line in content.lines() {
        let line = line.trim();

        if line.starts_with('[') && line.ends_with(']') {
            current_section = line[1..line.len() - 1].trim().to_string();
            continue;
        }

        if current_section.is_empty() || line.is_empty() || line.starts_with('#') {
            continue;
        }

        if let Some((key, value)) = line.split_once('=') {
            if key.contains('[') {
                continue;
            }
            let k = key.trim().to_string();
            let v = value.trim().to_string();

            if current_section == "Desktop Entry" {
                main_fields.insert(k, v);
            } else if let Some(action_id) = current_section.strip_prefix("Desktop Action ") {
                action_fields
                    .entry(action_id.trim().to_string())
                    .or_default()
                    .insert(k, v);
            }
        }
    }

    let entry_type = main_fields
        .get("Type")
        .map(String::as_str)
        .unwrap_or("Application");
    if entry_type != "Application"
        || parse_bool(&main_fields, "NoDisplay")
        || parse_bool(&main_fields, "Hidden")
        || !desktop_is_visible(&main_fields)
    {
        return None;
    }

    if let Some(try_exec) = main_fields.get("TryExec")
        && !try_exec_available(try_exec)
    {
        return None;
    }

    let name = main_fields.get("Name")?.clone();
    let dbus_activatable = parse_bool(&main_fields, "DBusActivatable");
    let exec = main_fields
        .get("Exec")
        .and_then(|raw| parse_exec(raw))
        .unwrap_or_default();

    if !dbus_activatable && exec.is_empty() {
        return None;
    }

    let working_dir = main_fields
        .get("Path")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);

    let icon = main_fields.get("Icon").cloned();

    let mut actions = Vec::new();
    let action_ids: Vec<String> = if let Some(actions_str) = main_fields.get("Actions") {
        actions_str
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect()
    } else {
        action_fields.keys().cloned().collect()
    };

    for id in action_ids {
        if let Some(fields) = action_fields.get(&id) {
            let action_name = fields.get("Name").cloned().unwrap_or_else(|| id.clone());
            let action_exec = fields
                .get("Exec")
                .and_then(|raw| parse_exec(raw))
                .unwrap_or_default();

            if action_exec.is_empty() {
                continue;
            }

            let action_icon = fields.get("Icon").cloned().or_else(|| icon.clone());

            actions.push(AppAction {
                id,
                name: action_name,
                exec: action_exec,
                icon: action_icon,
            });
        }
    }

    Some(App {
        name,
        exec,
        icon,
        actions,
        terminal: parse_bool(&main_fields, "Terminal"),
        path: working_dir,
        dbus_activatable,
        desktop_file: path.to_path_buf(),
    })
}

pub fn scan_apps() -> Vec<App> {
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut apps = Vec::new();

    for dir in desktop_file_dirs() {
        for (path, desktop_id) in desktop_files(&dir, String::new()) {
            if !seen_ids.insert(desktop_id) {
                continue;
            }

            if let Some(app) = parse_desktop_file(&path) {
                apps.push(app);
            }
        }
    }

    apps.sort_by_key(|app| app.name.to_lowercase());
    apps
}

fn desktop_files(dir: &Path, prefix: String) -> Vec<(PathBuf, String)> {
    let mut files = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return files;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };

        if file_type.is_dir() {
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let nested_prefix = if prefix.is_empty() {
                name.to_string()
            } else {
                format!("{prefix}-{name}")
            };
            files.extend(desktop_files(&path, nested_prefix));
        } else if (file_type.is_file()
            || (file_type.is_symlink()
                && fs::metadata(&path).is_ok_and(|metadata| metadata.is_file())))
            && path.extension().and_then(|extension| extension.to_str()) == Some("desktop")
        {
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let desktop_id = if prefix.is_empty() {
                name.to_string()
            } else {
                format!("{prefix}-{name}")
            };
            files.push((path, desktop_id));
        }
    }

    files
}

fn expand_exec(app: &App) -> Vec<String> {
    let mut expanded = Vec::new();

    for arg in &app.exec {
        match arg.as_str() {
            "%f" | "%F" | "%u" | "%U" => {}
            "%i" => {
                if let Some(icon) = &app.icon {
                    expanded.push("--icon".to_string());
                    expanded.push(icon.clone());
                }
            }
            "%c" => expanded.push(app.name.clone()),
            "%k" => expanded.push(app.desktop_file.to_string_lossy().into_owned()),
            _ => expanded.push(arg.replace("%%", "%")),
        }
    }

    expanded
}

fn expand_action_exec(app: &App, action: &AppAction) -> Vec<String> {
    let mut expanded = Vec::new();
    let icon_to_use = action.icon.as_deref().or(app.icon.as_deref());

    for arg in &action.exec {
        match arg.as_str() {
            "%f" | "%F" | "%u" | "%U" => {}
            "%i" => {
                if let Some(icon) = icon_to_use {
                    expanded.push("--icon".to_string());
                    expanded.push(icon.to_string());
                }
            }
            "%c" => expanded.push(action.name.clone()),
            "%k" => expanded.push(app.desktop_file.to_string_lossy().into_owned()),
            _ => expanded.push(arg.replace("%%", "%")),
        }
    }

    expanded
}

fn terminal_program() -> Option<Vec<String>> {
    if let Ok(value) = env::var("TERMINAL")
        && let Some(parts) = parse_exec(&value)
        && parts
            .first()
            .is_some_and(|command| command_available(command))
    {
        return Some(parts);
    }

    [
        "foot",
        "ghostty",
        "wezterm",
        "kitty",
        "alacritty",
        "xfce4-terminal",
        "konsole",
        "gnome-terminal",
        "x-terminal-emulator",
        "xterm",
    ]
    .iter()
    .find(|command| command_available(command))
    .map(|command| vec![(*command).to_string()])
}

fn launch_terminal(exec: &[String], path: Option<&Path>) -> io::Result<Command> {
    let mut terminal = terminal_program()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no terminal emulator was found"))?;
    let command = terminal.remove(0);
    let mut process = Command::new(command);
    process.args(terminal);

    let command_name = process.get_program().to_string_lossy();
    if command_name.ends_with("gnome-terminal") {
        process.arg("--");
    } else if command_name.ends_with("xfce4-terminal") {
        process.arg("--execute");
    } else {
        process.arg("-e");
    }

    process.args(exec);
    if let Some(path) = path {
        process.current_dir(path);
    }
    Ok(process)
}

fn spawn_detached(mut command: Command) -> io::Result<()> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            command
                .pre_exec(|| {
                    if libc_setsid() == -1 {
                        return Err(io::Error::last_os_error());
                    }
                    Ok(())
                })
                .spawn()?
        };
    }

    #[cfg(not(unix))]
    {
        command.spawn()?;
    }

    Ok(())
}

pub fn launch(app: &App) -> io::Result<()> {
    if app.dbus_activatable {
        let mut command = Command::new("gio");
        command.arg("launch").arg(&app.desktop_file);
        if let Some(path) = &app.path {
            command.current_dir(path);
        }
        return spawn_detached(command);
    }

    let exec = expand_exec(app);
    let Some((command, args)) = exec.split_first() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "desktop entry has no executable command",
        ));
    };

    if app.terminal {
        return spawn_detached(launch_terminal(&exec, app.path.as_deref())?);
    }

    let mut process = Command::new(command);
    process.args(args);
    if let Some(path) = &app.path {
        process.current_dir(path);
    }
    spawn_detached(process)
}

pub fn launch_action(app: &App, action: &AppAction) -> io::Result<()> {
    let exec = expand_action_exec(app, action);
    let Some((command, args)) = exec.split_first() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "desktop action has no executable command",
        ));
    };

    if app.terminal {
        return spawn_detached(launch_terminal(&exec, app.path.as_deref())?);
    }

    let mut process = Command::new(command);
    process.args(args);
    if let Some(path) = &app.path {
        process.current_dir(path);
    }
    spawn_detached(process)
}

#[cfg(unix)]
fn libc_setsid() -> i32 {
    unsafe extern "C" {
        fn setsid() -> i32;
    }
    unsafe { setsid() }
}

#[cfg(test)]
mod tests {
    use super::{parse_desktop_file, parse_exec, scan_apps, valid_field_codes};
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn parses_desktop_exec_quoting() {
        assert_eq!(
            parse_exec("\"/tmp/my app\" \"two words\" %%").unwrap(),
            vec!["/tmp/my app", "two words", "%%"]
        );
        assert_eq!(
            parse_exec("flatpak 'run' '%U'").unwrap(),
            vec!["flatpak", "run", "%U"]
        );
    }

    #[test]
    fn validates_field_codes() {
        assert!(valid_field_codes("foo%%bar"));
        assert!(valid_field_codes("%c"));
        assert!(!valid_field_codes("foo%c"));
        assert!(!valid_field_codes("%x"));
        assert!(!valid_field_codes("%"));
    }

    #[test]
    fn parses_launch_metadata() {
        let path = PathBuf::from(format!(
            "/tmp/mylauncher-desktop-test-{}.desktop",
            std::process::id()
        ));
        fs::write(
            &path,
            "[Desktop Entry]\nType=Application\nName=Example\nExec=\"/tmp/my app\" --value \"two words\"\nTerminal=true\nPath=/tmp\n",
        )
        .unwrap();

        let app = parse_desktop_file(&path).unwrap();
        assert_eq!(app.name, "Example");
        assert_eq!(app.exec, vec!["/tmp/my app", "--value", "two words"]);
        assert!(app.terminal);
        assert_eq!(app.path, Some(PathBuf::from("/tmp")));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn parses_desktop_actions() {
        let path = PathBuf::from(format!(
            "/tmp/mylauncher-actions-test-{}.desktop",
            std::process::id()
        ));
        fs::write(
            &path,
            "[Desktop Entry]\nType=Application\nName=Browser\nExec=browser %u\nActions=new-window;private;\n\n[Desktop Action new-window]\nName=New Window\nExec=browser --new-window\n\n[Desktop Action private]\nName=Private Window\nExec=browser --incognito\n",
        )
        .unwrap();

        let app = parse_desktop_file(&path).unwrap();
        assert_eq!(app.actions.len(), 2);
        assert_eq!(app.actions[0].id, "new-window");
        assert_eq!(app.actions[0].name, "New Window");
        assert_eq!(app.actions[0].exec, vec!["browser", "--new-window"]);
        assert_eq!(app.actions[1].id, "private");
        assert_eq!(app.actions[1].name, "Private Window");
        assert_eq!(app.actions[1].exec, vec!["browser", "--incognito"]);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn scans_real_apps_speed() {
        let start = std::time::Instant::now();
        let apps = scan_apps();
        let elapsed = start.elapsed();
        println!("scanned {} apps in {:?}", apps.len(), elapsed);
    }
}
