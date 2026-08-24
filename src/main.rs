mod apps;
mod history;
mod pratt;
mod recents;
mod settings;
mod storage;
mod timers;

use crossterm::cursor;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind};
use crossterm::style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor};
use crossterm::terminal::{self, ClearType};
use crossterm::{execute, queue};
use std::io::{self, Write};
use std::time::Duration;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use apps::App;
use settings::{DisplayMode, Settings};
use timers::SharedTimers;

struct Palette {
    bg: Color,
    fg: Color,
    dim: Color,
    accent: Color,
    sel_fg: Color,
    danger: Color,
}

const CUSTOM: Palette = Palette {
    bg: Color::Rgb {
        r: 29,
        g: 27,
        b: 25,
    },
    fg: Color::Rgb {
        r: 232,
        g: 228,
        b: 220,
    },
    dim: Color::Rgb {
        r: 138,
        g: 133,
        b: 122,
    },
    accent: Color::Rgb {
        r: 215,
        g: 165,
        b: 58,
    },
    sel_fg: Color::Rgb {
        r: 29,
        g: 27,
        b: 25,
    },
    danger: Color::Rgb {
        r: 224,
        g: 85,
        b: 85,
    },
};

const RAW: Palette = Palette {
    bg: Color::Reset,
    fg: Color::Reset,
    dim: Color::DarkGrey,
    accent: Color::Yellow,
    sel_fg: Color::Black,
    danger: Color::Red,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    Apps,
    Actions { action_selected: usize },
}

struct State {
    query: String,
    cursor: usize,
    all_apps: Vec<App>,
    selected: usize,
    history_selected: usize,
    timers: SharedTimers,
    math_history: Vec<String>,
    settings: Settings,
    recents: Vec<String>,
    launch_error: Option<String>,
    view_mode: ViewMode,
}

fn main() -> io::Result<()> {
    let all_apps = apps::scan_apps();
    let timers = timers::load_shared();

    timers::spawn_ticker(timers.clone());

    let math_history = history::load();
    let settings = settings::load();
    let recents = recents::load();

    let mut state = State {
        query: String::new(),
        cursor: 0,
        all_apps,
        selected: 0,
        history_selected: 0,
        timers,
        math_history,
        settings,
        recents,
        launch_error: None,
        view_mode: ViewMode::Apps,
    };

    run(&mut state)
}

fn run(state: &mut State) -> io::Result<()> {
    let mut stdout = io::stdout();

    terminal::enable_raw_mode()?;
    let _guard = TerminalGuard;

    execute!(
        stdout,
        terminal::EnterAlternateScreen,
        cursor::Show,
        cursor::SetCursorStyle::BlinkingBlock,
        event::EnableMouseCapture
    )?;

    event_loop(state, &mut stdout)
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let mut stdout = io::stdout();
        let _ = execute!(
            stdout,
            event::DisableMouseCapture,
            cursor::SetCursorStyle::DefaultUserShape,
            cursor::Show,
            terminal::LeaveAlternateScreen
        );
        let _ = terminal::disable_raw_mode();
    }
}

fn event_loop(state: &mut State, stdout: &mut io::Stdout) -> io::Result<()> {
    let mut needs_redraw = true;
    let mut last_size = terminal::size().unwrap_or((0, 0));
    let mut last_timer_check = std::time::Instant::now();

    loop {
        let current_size = terminal::size().unwrap_or(last_size);
        if current_size != last_size {
            last_size = current_size;
            needs_redraw = true;
        }

        if needs_redraw {
            draw(state, stdout)?;
            needs_redraw = false;
        }

        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Resize(w, h) => {
                    last_size = (w, h);
                    needs_redraw = true;
                }

                Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                    needs_redraw = true;
                    let ctrl = key_event.modifiers.contains(KeyModifiers::CONTROL);

                    match key_event.code {
                        KeyCode::Esc => {
                            if state.view_mode != ViewMode::Apps {
                                state.view_mode = ViewMode::Apps;
                            } else {
                                break;
                            }
                        }

                        KeyCode::Char('c') if ctrl => {
                            break;
                        }

                        KeyCode::Char('u') if ctrl => {
                            state.query.clear();
                            state.cursor = 0;
                            state.selected = 0;
                            state.history_selected = 0;
                            state.launch_error = None;
                            state.view_mode = ViewMode::Apps;
                        }

                        KeyCode::Char('w') if ctrl => {
                            while state.cursor > 0 {
                                let prev = cursor_left(&state.query, state.cursor);
                                let was_space = state.query[prev..state.cursor]
                                    .chars()
                                    .next()
                                    .is_some_and(char::is_whitespace);
                                state.query.remove(prev);
                                state.cursor = prev;

                                if was_space {
                                    break;
                                }
                            }
                            state.selected = 0;
                            state.history_selected = 0;
                            state.launch_error = None;
                            state.view_mode = ViewMode::Apps;
                        }

                        KeyCode::Left => {
                            if state.view_mode != ViewMode::Apps {
                                state.view_mode = ViewMode::Apps;
                            } else {
                                state.cursor = cursor_left(&state.query, state.cursor);
                            }
                        }

                        KeyCode::Right => {
                            if state.view_mode == ViewMode::Apps
                                && !is_timer_mode(&state.query)
                                && !is_math_mode(&state.query)
                                && !is_settings_mode(&state.query)
                            {
                                let matches = matching_apps(state);
                                if let Some(app) = matches.get(state.selected) {
                                    if !app.actions.is_empty() {
                                        state.view_mode = ViewMode::Actions {
                                            action_selected: 0,
                                        };
                                    } else {
                                        state.cursor = cursor_right(&state.query, state.cursor);
                                    }
                                }
                            } else {
                                state.cursor = cursor_right(&state.query, state.cursor);
                            }
                        }

                        KeyCode::Tab => {
                            if state.view_mode == ViewMode::Apps
                                && !is_timer_mode(&state.query)
                                && !is_math_mode(&state.query)
                                && !is_settings_mode(&state.query)
                            {
                                let matches = matching_apps(state);
                                if let Some(app) = matches.get(state.selected) {
                                    if !app.actions.is_empty() {
                                        state.view_mode = ViewMode::Actions {
                                            action_selected: 0,
                                        };
                                    }
                                }
                            }
                        }

                        KeyCode::BackTab => {
                            if state.view_mode != ViewMode::Apps {
                                state.view_mode = ViewMode::Apps;
                            }
                        }

                        KeyCode::Char('l') if ctrl => {
                            if state.view_mode == ViewMode::Apps {
                                let matches = matching_apps(state);
                                if let Some(app) = matches.get(state.selected) {
                                    if !app.actions.is_empty() {
                                        state.view_mode = ViewMode::Actions {
                                            action_selected: 0,
                                        };
                                    }
                                }
                            }
                        }

                        KeyCode::Char('h') if ctrl || state.view_mode != ViewMode::Apps => {
                            if state.view_mode != ViewMode::Apps {
                                state.view_mode = ViewMode::Apps;
                            }
                        }

                        KeyCode::Char('k') if ctrl || state.view_mode != ViewMode::Apps => {
                            move_selection(state, -1);
                        }

                        KeyCode::Char('p') if ctrl => {
                            move_selection(state, -1);
                        }

                        KeyCode::Char('j') if ctrl || state.view_mode != ViewMode::Apps => {
                            move_selection(state, 1);
                        }

                        KeyCode::Char('n') if ctrl => {
                            move_selection(state, 1);
                        }

                        KeyCode::Home => {
                            state.cursor = 0;
                        }

                        KeyCode::End => {
                            state.cursor = state.query.len();
                        }

                        KeyCode::Char(c) => {
                            state.view_mode = ViewMode::Apps;
                            state.query.insert(state.cursor, c);
                            state.cursor += c.len_utf8();
                            state.selected = 0;
                            state.history_selected = 0;
                            state.launch_error = None;
                        }

                        KeyCode::Backspace => {
                            if state.view_mode != ViewMode::Apps {
                                state.view_mode = ViewMode::Apps;
                            } else if state.cursor > 0 {
                                let prev = cursor_left(&state.query, state.cursor);
                                state.query.remove(prev);
                                state.cursor = prev;
                                state.selected = 0;
                                state.history_selected = 0;
                                state.launch_error = None;
                            }
                        }

                        KeyCode::Delete => {
                            if state.view_mode != ViewMode::Apps {
                                state.view_mode = ViewMode::Apps;
                            } else if state.cursor < state.query.len() {
                                state.query.remove(state.cursor);
                                state.selected = 0;
                                state.history_selected = 0;
                                state.launch_error = None;
                            }
                        }

                        KeyCode::Up => {
                            move_selection(state, -1);
                        }

                        KeyCode::Down => {
                            move_selection(state, 1);
                        }

                        KeyCode::PageUp => {
                            move_selection(state, -5);
                        }

                        KeyCode::PageDown => {
                            move_selection(state, 5);
                        }

                        KeyCode::Enter if handle_enter(state)? => {
                            break;
                        }

                        _ => {}
                    }
                }

                Event::Mouse(mouse_event) => match mouse_event.kind {
                    MouseEventKind::ScrollUp => {
                        move_selection(state, -1);
                        needs_redraw = true;
                    }
                    MouseEventKind::ScrollDown => {
                        move_selection(state, 1);
                        needs_redraw = true;
                    }
                    _ => {}
                },

                _ => {}
            }
        } else if state.settings.timers && last_timer_check.elapsed() >= Duration::from_millis(500) {
            last_timer_check = std::time::Instant::now();
            let list = state.timers.lock().unwrap();
            let has_active = list.iter().any(|t| !t.finished);
            drop(list);
            if has_active {
                needs_redraw = true;
            }
        }
    }

    Ok(())
}

fn move_selection(state: &mut State, delta: i32) {
    if is_timer_mode(&state.query) || is_settings_mode(&state.query) {
        return;
    }

    if let ViewMode::Actions { action_selected } = state.view_mode {
        let matches = matching_apps(state);
        if let Some(app) = matches.get(state.selected) {
            let total_actions = 1 + app.actions.len();
            let new_selected = if total_actions == 0 {
                0
            } else if delta < 0 {
                let abs = delta.unsigned_abs() as usize;
                action_selected.saturating_sub(abs)
            } else {
                let count = delta as usize;
                (action_selected + count).min(total_actions - 1)
            };
            state.view_mode = ViewMode::Actions {
                action_selected: new_selected,
            };
        }
        return;
    }

    if is_math_mode(&state.query) {
        let len = state.math_history.len();

        if len == 0 {
            state.history_selected = 0;
            return;
        }

        if delta < 0 {
            let abs = delta.unsigned_abs() as usize;
            state.history_selected = state.history_selected.saturating_sub(abs);
        } else {
            let count = delta as usize;
            state.history_selected = (state.history_selected + count).min(len - 1);
        }

        return;
    }

    let matches = matching_apps(state);

    if matches.is_empty() {
        state.selected = 0;
        return;
    }

    if delta < 0 {
        let abs = delta.unsigned_abs() as usize;
        state.selected = state.selected.saturating_sub(abs);
    } else {
        let count = delta as usize;
        state.selected = (state.selected + count).min(matches.len() - 1);
    }
}

fn handle_enter(state: &mut State) -> io::Result<bool> {
    state.launch_error = None;

    if let ViewMode::Actions { action_selected } = state.view_mode {
        let matches = matching_apps(state);
        if let Some(app) = matches.get(state.selected) {
            let launch_res = if action_selected == 0 {
                apps::launch(app)
            } else if let Some(action) = app.actions.get(action_selected - 1) {
                apps::launch_action(app, action)
            } else {
                apps::launch(app)
            };

            match launch_res {
                Ok(()) => {
                    state.recents = recents::record(&app.name);
                    return Ok(true);
                }
                Err(error) => {
                    state.launch_error = Some(format!("launch failed: {error}"));
                    state.view_mode = ViewMode::Apps;
                    return Ok(false);
                }
            }
        }
        state.view_mode = ViewMode::Apps;
        return Ok(false);
    }

    let query = state.query.trim().to_string();

    if let Some(command) = timer_command(&query) {
        if command.is_empty() {
            return Ok(false);
        }

        if let Some(rest) = strip_prefix_word(command, &["clear"]) {
            if let Err(error) = timers::clear_command(&state.timers, rest) {
                state.launch_error = Some(format!("timer failed: {error}"));
            }

            state.query = "t: ".to_string();
            state.cursor = state.query.len();
            state.selected = 0;

            return Ok(false);
        }

        if let Some((duration, label)) = timers::parse_timer_input(command) {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            if let Some(end_unix) = now.checked_add(duration.as_secs()) {
                if let Err(error) = timers::add(
                    &state.timers,
                    timers::Timer {
                        label,
                        end_unix,
                        finished: false,
                    },
                ) {
                    state.launch_error = Some(format!("timer failed: {error}"));
                } else {
                    state.query = "t: ".to_string();
                    state.cursor = state.query.len();
                    state.selected = 0;
                }
            } else {
                state.launch_error = Some("timer duration is too large".to_string());
            }
        }

        return Ok(false);
    }

    if let Some(expression) = math_command(&query) {
        if let Some(rest) = strip_prefix_word(expression, &["clear", "delete"]) {
            let rest = rest.trim();

            if rest.is_empty() || rest.eq_ignore_ascii_case("all") {
                state.math_history.clear();
            } else if let Ok(number) = rest.parse::<usize>()
                && number > 0
                && number <= state.math_history.len()
            {
                let index = state.math_history.len() - number;
                state.math_history.remove(index);
            }

            if let Err(error) = history::save(&state.math_history) {
                state.launch_error = Some(format!("history failed: {error}"));
            }

            if state.history_selected >= state.math_history.len() {
                state.history_selected = state.math_history.len().saturating_sub(1);
            }

            state.query = "m: ".to_string();
            state.cursor = state.query.len();

            return Ok(false);
        }

        if expression.is_empty() {
            let len = state.math_history.len();

            if len > 0 {
                let index = len - 1 - state.history_selected.min(len - 1);

                if let Some((expr, _)) = state.math_history[index].rsplit_once(" = ") {
                    state.query = format!("m: {}", expr);
                    state.cursor = state.query.len();
                    state.history_selected = 0;
                }
            }

            return Ok(false);
        }

        if let Ok(value) = pratt::calculate(expression) {
            let entry = format!("{} = {}", expression, value);

            match history::append(&entry) {
                Ok(()) => state.math_history.push(entry),
                Err(error) => {
                    state.launch_error = Some(format!("history failed: {error}"));
                }
            }

            state.query = "m: ".to_string();
            state.cursor = state.query.len();
            state.history_selected = 0;
        }

        return Ok(false);
    }

    if let Some(command) = settings_command(&query) {
        let mut parts = command.split_whitespace();

        if let (Some(index), Some(value), None) = (parts.next(), parts.next(), parts.next()) {
            let applied = match index {
                "1" | "2" | "3" => {
                    let flag = match value {
                        "true" => true,
                        "false" => false,
                        _ => return Ok(false),
                    };

                    match index {
                        "1" => state.settings.background = flag,
                        "2" => state.settings.footer = flag,
                        _ => state.settings.timers = flag,
                    }

                    true
                }
                "4" => match DisplayMode::from_str(value) {
                    Some(mode) => {
                        state.settings.display = mode;
                        true
                    }
                    None => false,
                },
                _ => false,
            };

            if applied {
                if let Err(error) = settings::save(&state.settings) {
                    state.launch_error = Some(format!("settings failed: {error}"));
                }
                state.query = "s: ".to_string();
                state.cursor = state.query.len();
            }
        }

        return Ok(false);
    }

    let matches = matching_apps(state);

    if state.selected >= matches.len() {
        state.selected = matches.len().saturating_sub(1);
    }

    if let Some(app) = matches.get(state.selected) {
        match apps::launch(app) {
            Ok(()) => {
                state.recents = recents::record(&app.name);
                return Ok(true);
            }
            Err(error) => {
                state.launch_error = Some(format!("launch failed: {error}"));
            }
        }
    }

    Ok(false)
}

fn timer_command(input: &str) -> Option<&str> {
    let trimmed = input.trim();

    if trimmed == "t:" {
        return Some("");
    }

    if let Some(rest) = trimmed.strip_prefix("t:") {
        return Some(rest.trim());
    }

    None
}

fn math_command(input: &str) -> Option<&str> {
    let trimmed = input.trim();

    if trimmed == "m:" {
        return Some("");
    }

    if let Some(rest) = trimmed.strip_prefix("m:") {
        return Some(rest.trim());
    }

    None
}

fn settings_command(input: &str) -> Option<&str> {
    let trimmed = input.trim();

    if trimmed == "s:" {
        return Some("");
    }

    if let Some(rest) = trimmed.strip_prefix("s:") {
        return Some(rest.trim());
    }

    None
}

fn is_timer_mode(input: &str) -> bool {
    timer_command(input).is_some()
}

fn is_math_mode(input: &str) -> bool {
    math_command(input).is_some()
}

fn is_settings_mode(input: &str) -> bool {
    settings_command(input).is_some()
}

fn strip_prefix_word<'a>(input: &'a str, words: &[&str]) -> Option<&'a str> {
    for word in words {
        if let Some(rest) = input.strip_prefix(word)
            && (rest.is_empty() || rest.chars().next().is_some_and(char::is_whitespace))
        {
            return Some(rest.trim_start());
        }
    }

    None
}

fn match_score(name_lower: &str, query: &str) -> Option<usize> {
    if query.is_empty() {
        return Some(0);
    }

    if name_lower.starts_with(query) {
        return Some(0);
    }

    if name_lower.contains(query) {
        return Some(1);
    }

    let mut chars = name_lower.chars();

    for want in query.chars() {
        loop {
            match chars.next() {
                Some(c) if c == want => break,
                Some(_) => continue,
                None => return None,
            }
        }
    }

    Some(2)
}

fn matching_apps(state: &State) -> Vec<App> {
    let query = state.query.trim().to_lowercase();

    if query.is_empty() && state.settings.display == DisplayMode::None {
        return Vec::new();
    }

    let mut scored: Vec<(usize, App)> = state
        .all_apps
        .iter()
        .filter_map(|app| {
            match_score(&app.name.to_lowercase(), &query).map(|score| (score, app.clone()))
        })
        .collect();

    match state.settings.display {
        DisplayMode::Recent if query.is_empty() => {
            scored.sort_by_key(|(_, app)| recency_rank(&state.recents, &app.name));
        }
        DisplayMode::Recent => {
            scored.sort_by_key(|(score, app)| (*score, recency_rank(&state.recents, &app.name)));
        }
        _ => {
            scored.sort_by_key(|(score, _)| *score);
        }
    }

    scored.into_iter().map(|(_, app)| app).collect()
}

fn recency_rank(recents: &[String], name: &str) -> usize {
    recents.iter().position(|n| n == name).unwrap_or(usize::MAX)
}

fn cursor_left(query: &str, cursor: usize) -> usize {
    query[..cursor]
        .char_indices()
        .next_back()
        .map(|(i, _)| i)
        .unwrap_or(0)
}

fn cursor_right(query: &str, cursor: usize) -> usize {
    query[cursor..]
        .chars()
        .next()
        .map(|c| cursor + c.len_utf8())
        .unwrap_or(cursor)
}

fn fit(s: &str, width: usize) -> String {
    if s.width() <= width {
        s.to_string()
    } else {
        let (mut out, _) = split_cells(s, width.saturating_sub(1));
        out.push('…');
        out
    }
}

fn split_cells(s: &str, width: usize) -> (String, String) {
    if width == 0 {
        return (String::new(), s.to_string());
    }

    let mut used = 0;
    let mut split_at = 0;

    for (index, ch) in s.char_indices() {
        let char_width = ch.width().unwrap_or(0);
        if used > 0 && used + char_width > width {
            split_at = index;
            break;
        }
        used += char_width;
        split_at = index + ch.len_utf8();
        if used >= width {
            break;
        }
    }

    (s[..split_at].to_string(), s[split_at..].to_string())
}

fn skip_cells(s: &str, width: usize) -> &str {
    let mut used = 0;

    for (index, ch) in s.char_indices() {
        if used >= width {
            return &s[index..];
        }
        used += ch.width().unwrap_or(0);
        if used >= width {
            return &s[index + ch.len_utf8()..];
        }
    }

    ""
}

fn take_cells(s: &str, width: usize) -> String {
    split_cells(skip_cells(s, 0), width).0
}

fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();

    for word in text.split_whitespace() {
        let mut word = word.to_string();

        while word.width() > width {
            if !cur.is_empty() {
                lines.push(std::mem::take(&mut cur));
            }

            let (head, rest) = split_cells(&word, width);
            lines.push(head);
            word = rest;
        }

        if cur.is_empty() {
            cur = word;
        } else if cur.width() + 1 + word.width() <= width {
            cur.push(' ');
            cur.push_str(&word);
        } else {
            lines.push(std::mem::take(&mut cur));
            cur = word;
        }
    }

    if !cur.is_empty() {
        lines.push(cur);
    }

    if lines.is_empty() {
        lines.push(String::new());
    }

    lines
}

fn fill_background(stdout: &mut io::Stdout, cols: u16, rows: u16, bg: Color) -> io::Result<()> {
    if bg == Color::Reset {
        return Ok(());
    }

    let line = " ".repeat(cols as usize);

    queue!(stdout, SetBackgroundColor(bg))?;

    for y in 0..rows {
        queue!(stdout, cursor::MoveTo(0, y), Print(&line))?;
    }

    queue!(stdout, ResetColor)?;

    Ok(())
}

struct ListRow {
    text: String,
    badge: Option<String>,
    color: Color,
}

#[allow(clippy::too_many_arguments)]
fn draw_list(
    stdout: &mut io::Stdout,
    row: &mut u16,
    term_rows: u16,
    cols: u16,
    bottom_lines: u16,
    palette: &Palette,
    items: &[ListRow],
    selected: Option<usize>,
) -> io::Result<()> {
    if items.is_empty() {
        return Ok(());
    }

    let width = cols as usize;

    let wrapped: Vec<Vec<String>> = items
        .iter()
        .map(|it| {
            let right_pad = if it.badge.is_some() { 4 } else { 2 };
            let text_width = width.saturating_sub(2 + right_pad).max(1);
            wrap(&it.text, text_width)
        })
        .collect();

    let available = term_rows.saturating_sub(*row).saturating_sub(bottom_lines) as usize;

    if available == 0 {
        return Ok(());
    }

    let total: usize = wrapped.iter().map(|w| w.len()).sum();
    let sel = selected.unwrap_or(0).min(items.len() - 1);

    let (first, last) = if total <= available {
        (0, items.len().saturating_sub(1))
    } else {
        let mut first = sel;
        let mut last = sel;
        let mut used = wrapped[sel].len();

        while last + 1 < items.len() {
            let next_len = wrapped[last + 1].len();
            let needed_more = if last + 2 < items.len() { 1 } else { 0 };
            if used + next_len + needed_more <= available {
                last += 1;
                used += next_len;
            } else {
                break;
            }
        }

        while first > 0 {
            let prev_len = wrapped[first - 1].len();
            let needed_more = if last + 1 < items.len() { 1 } else { 0 };
            if used + prev_len + needed_more <= available {
                first -= 1;
                used += prev_len;
            } else {
                break;
            }
        }

        (first, last)
    };

    let mut budget = available;
    let mut y = *row;

    for i in first..=last {
        let is_sel = selected == Some(i);
        let row_bg = if is_sel {
            palette.accent
        } else {
            palette.bg
        };
        let fg_color = if is_sel {
            palette.sel_fg
        } else {
            items[i].color
        };
        let dim_color = if is_sel {
            palette.sel_fg
        } else {
            palette.dim
        };

        for (line_idx, line) in wrapped[i].iter().enumerate() {
            if budget == 0 {
                break;
            }

            queue!(stdout, cursor::MoveTo(0, y))?;

            if line_idx == 0 {
                queue!(
                    stdout,
                    SetBackgroundColor(row_bg),
                    SetForegroundColor(fg_color),
                    Print(format!("  {}", line)),
                )?;

                let used_width = 2 + line.width();
                if let Some(ref badge) = items[i].badge {
                    let badge_w = badge.width();
                    let pad = width.saturating_sub(used_width + badge_w + 2);
                    queue!(
                        stdout,
                        Print(" ".repeat(pad)),
                        SetForegroundColor(dim_color),
                        Print(badge),
                        Print("  "),
                        ResetColor,
                    )?;
                } else {
                    let pad = width.saturating_sub(used_width);
                    queue!(stdout, Print(" ".repeat(pad)), ResetColor)?;
                }
            } else {
                let used_width = 2 + line.width();
                let pad = width.saturating_sub(used_width);
                queue!(
                    stdout,
                    SetBackgroundColor(row_bg),
                    SetForegroundColor(fg_color),
                    Print("  "),
                    Print(line),
                    Print(" ".repeat(pad)),
                    ResetColor,
                )?;
            }

            y += 1;
            budget -= 1;
        }
    }

    let more = items.len().saturating_sub(last + 1);

    if more > 0 && budget > 0 {
        queue!(
            stdout,
            cursor::MoveTo(0, y),
            SetForegroundColor(palette.dim),
            SetBackgroundColor(palette.bg),
            Print(format!("  … {} more", more)),
            ResetColor,
        )?;

        y += 1;
    }

    *row = y;

    Ok(())
}

fn draw_strip(
    stdout: &mut io::Stdout,
    start_y: u16,
    palette: &Palette,
    label_text: &str,
    body_col: u16,
    lines: &[String],
) -> io::Result<()> {
    queue!(
        stdout,
        cursor::MoveTo(0, start_y),
        SetForegroundColor(palette.accent),
        SetBackgroundColor(palette.bg),
        Print(label_text),
        ResetColor,
    )?;

    for (i, line) in lines.iter().enumerate() {
        queue!(
            stdout,
            cursor::MoveTo(body_col, start_y + i as u16),
            SetForegroundColor(palette.dim),
            SetBackgroundColor(palette.bg),
            Print(line),
            ResetColor,
        )?;
    }

    Ok(())
}

fn draw(state: &mut State, stdout: &mut io::Stdout) -> io::Result<()> {
    let (cols, term_rows) = terminal::size()?;

    let palette = if state.settings.background {
        &CUSTOM
    } else {
        &RAW
    };

    queue!(
        stdout,
        terminal::Clear(ClearType::All),
        cursor::MoveTo(0, 0)
    )?;

    fill_background(stdout, cols, term_rows, palette.bg)?;

    let prompt_width = 2usize;
    let visible = (cols as usize).saturating_sub(prompt_width + 1);
    let query_len = state.query.width();
    let cursor = state.cursor.min(state.query.len());
    let cursor_chars = state.query[..cursor].width();

    let start = if query_len <= visible {
        0
    } else {
        let mut s = cursor_chars.saturating_sub(visible.saturating_sub(1));

        if s + visible > query_len {
            s = query_len - visible;
        }

        s
    };

    let query_trim = state.query.trim().to_string();

    let is_timer = is_timer_mode(&query_trim);
    let is_math = is_math_mode(&query_trim);
    let is_settings = is_settings_mode(&query_trim);

    let matches = if !is_timer && !is_math && !is_settings {
        Some(matching_apps(state))
    } else {
        None
    };

    if let ViewMode::Actions { .. } = state.view_mode {
        let app_name = matches
            .as_ref()
            .and_then(|m| m.get(state.selected))
            .map(|a| a.name.as_str())
            .unwrap_or("App");
        let header_text = format!("{} › Launch Options", app_name);
        let visible_header = fit(&header_text, visible);

        queue!(
            stdout,
            cursor::MoveTo(0, 0),
            SetForegroundColor(palette.accent),
            SetBackgroundColor(palette.bg),
            Print("> "),
            SetForegroundColor(palette.fg),
            Print(&visible_header),
            ResetColor,
        )?;
    } else {
        let visible_query = take_cells(skip_cells(&state.query, start), visible);

        queue!(
            stdout,
            cursor::MoveTo(0, 0),
            SetForegroundColor(palette.accent),
            SetBackgroundColor(palette.bg),
            Print("> "),
            SetForegroundColor(palette.fg),
            Print(&visible_query),
            ResetColor,
        )?;
    }

    queue!(
        stdout,
        cursor::MoveTo(0, 1),
        SetForegroundColor(palette.dim),
        SetBackgroundColor(palette.bg),
        Print("─".repeat(cols as usize)),
        ResetColor,
    )?;

    let (label, segments): (&str, Vec<&str>) = if let ViewMode::Actions { .. } = state.view_mode {
        (
            "actions",
            vec!["↑↓ select", "enter launch", "←/esc back"],
        )
    } else if is_timer {
        (
            "timers",
            vec!["5m coffee", "clear [all|n|done]", "esc quit"],
        )
    } else if is_math {
        (
            "calc",
            vec!["↑↓ recall", "enter calculate", "clear [all|n]", "esc quit"],
        )
    } else if is_settings {
        (
            "settings",
            vec!["1-3 true|false", "4 none|apps|recent", "esc quit"],
        )
    } else {
        (
            "search",
            vec![
                "↑↓ select",
                "enter open",
                "→ options",
                "t: timers",
                "m: calc",
                "s: settings",
                "esc quit",
            ],
        )
    };

    let label_text = format!("  {}", label);
    let body_col = label_text.chars().count() + 2;
    let body_width = (cols as usize).saturating_sub(body_col);

    let body = segments.join(" · ");

    let footer_wrapped = if state.settings.footer {
        Some(wrap(&body, body_width))
    } else {
        None
    };

    let footer_lines = footer_wrapped.as_ref().map_or(0, |w| w.len().max(1) as u16);

    let timers_label_text = "  timers".to_string();
    let timers_body_col = timers_label_text.chars().count() + 2;
    let timers_body_width = (cols as usize).saturating_sub(timers_body_col);

    let timer_wrapped = if state.settings.timers {
        let list = state.timers.lock().unwrap();
        let active: Vec<String> = list
            .iter()
            .filter(|t| !t.finished)
            .map(|t| format!("{} {}", timers::format_remaining(t.remaining()), t.label))
            .collect();
        drop(list);

        if active.is_empty() {
            None
        } else {
            Some(wrap(&active.join(" · "), timers_body_width))
        }
    } else {
        None
    };

    let timer_lines = timer_wrapped.as_ref().map_or(0, |w| w.len().max(1) as u16);

    let error_lines = state.launch_error.is_some() as u16;
    let bottom_lines = footer_lines + timer_lines + error_lines;

    let mut row = 2u16;

    if let ViewMode::Actions { action_selected } = state.view_mode {
        if let Some(m) = &matches
            && let Some(app) = m.get(state.selected)
        {
            draw_actions_mode(
                app,
                action_selected,
                stdout,
                &mut row,
                term_rows,
                cols,
                bottom_lines,
                palette,
            )?;
        }
    } else if let Some(command) = timer_command(&query_trim) {
        draw_timer_mode(
            state,
            stdout,
            command,
            &mut row,
            term_rows,
            cols,
            bottom_lines,
            palette,
        )?;
    } else if let Some(expression) = math_command(&query_trim) {
        draw_math_mode(
            state,
            stdout,
            expression,
            &mut row,
            term_rows,
            cols,
            bottom_lines,
            palette,
        )?;
    } else if is_settings {
        draw_settings_mode(
            state,
            stdout,
            &mut row,
            term_rows,
            cols,
            bottom_lines,
            palette,
        )?;
    } else if let Some(m) = &matches {
        draw_app_results(
            state,
            stdout,
            m,
            &mut row,
            term_rows,
            cols,
            bottom_lines,
            palette,
        )?;
    }

    if let Some(lines) = &timer_wrapped
        && term_rows >= bottom_lines
    {
        draw_strip(
            stdout,
            term_rows - bottom_lines,
            palette,
            &timers_label_text,
            timers_body_col as u16,
            lines,
        )?;
    }

    if let Some(error) = &state.launch_error
        && term_rows >= bottom_lines
    {
        queue!(
            stdout,
            cursor::MoveTo(0, term_rows - footer_lines - error_lines),
            SetForegroundColor(palette.danger),
            SetBackgroundColor(palette.bg),
            Print(fit(&format!("  {error}"), cols as usize)),
            ResetColor,
        )?;
    }

    if let Some(lines) = &footer_wrapped
        && term_rows >= footer_lines
    {
        draw_strip(
            stdout,
            term_rows - footer_lines,
            palette,
            &label_text,
            body_col as u16,
            lines,
        )?;
    }

    if let ViewMode::Actions { .. } = state.view_mode {
        queue!(stdout, cursor::Hide)?;
    } else {
        let cursor_col = (prompt_width + cursor_chars.saturating_sub(start)) as u16;
        queue!(
            stdout,
            cursor::Show,
            cursor::MoveTo(cursor_col.min(cols.saturating_sub(1)), 0),
        )?;
    }

    stdout.flush()?;

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn draw_actions_mode(
    app: &App,
    action_selected: usize,
    stdout: &mut io::Stdout,
    row: &mut u16,
    term_rows: u16,
    cols: u16,
    bottom_lines: u16,
    palette: &Palette,
) -> io::Result<()> {
    let mut items = Vec::new();

    items.push(ListRow {
        text: format!("Launch {} (Default)", app.name),
        badge: None,
        color: palette.fg,
    });

    for action in &app.actions {
        items.push(ListRow {
            text: action.name.clone(),
            badge: None,
            color: palette.fg,
        });
    }

    draw_list(
        stdout,
        row,
        term_rows,
        cols,
        bottom_lines,
        palette,
        &items,
        Some(action_selected),
    )
}

#[allow(clippy::too_many_arguments)]
fn draw_timer_mode(
    state: &State,
    stdout: &mut io::Stdout,
    command: &str,
    row: &mut u16,
    term_rows: u16,
    cols: u16,
    bottom_lines: u16,
    palette: &Palette,
) -> io::Result<()> {
    if !command.is_empty()
        && let Some((duration, label)) = timers::parse_timer_input(command)
    {
        queue!(
            stdout,
            cursor::MoveTo(0, *row),
            SetForegroundColor(palette.accent),
            SetBackgroundColor(palette.bg),
            Print(fit(
                &format!("  [{}] {}", timers::format_remaining(duration), label),
                cols as usize,
            )),
            ResetColor,
        )?;

        *row += 2;
    }

    let list = state.timers.lock().unwrap();

    if list.is_empty() {
        queue!(
            stdout,
            cursor::MoveTo(0, *row),
            SetForegroundColor(palette.dim),
            SetBackgroundColor(palette.bg),
            Print("  no timers"),
            ResetColor,
        )?;

        *row += 1;

        return Ok(());
    }

    let items: Vec<ListRow> = list
        .iter()
        .rev()
        .enumerate()
        .map(|(index, timer)| {
            let (marker, color) = if timer.finished {
                ("done".to_string(), palette.danger)
            } else {
                (timers::format_remaining(timer.remaining()), palette.fg)
            };

            ListRow {
                text: format!("{}. [{}] {}", index + 1, marker, timer.label),
                badge: None,
                color,
            }
        })
        .collect();

    drop(list);

    draw_list(
        stdout,
        row,
        term_rows,
        cols,
        bottom_lines,
        palette,
        &items,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn draw_math_mode(
    state: &mut State,
    stdout: &mut io::Stdout,
    expression: &str,
    row: &mut u16,
    term_rows: u16,
    cols: u16,
    bottom_lines: u16,
    palette: &Palette,
) -> io::Result<()> {
    if !expression.is_empty() {
        if let Some(rest) = strip_prefix_word(expression, &["clear", "delete"]) {
            let message = if rest.is_empty() || rest.eq_ignore_ascii_case("all") {
                "  enter to clear all history"
            } else {
                "  enter to clear that entry"
            };

            queue!(
                stdout,
                cursor::MoveTo(0, *row),
                SetForegroundColor(palette.dim),
                SetBackgroundColor(palette.bg),
                Print(message),
                ResetColor,
            )?;
        } else {
            match pratt::calculate(expression) {
                Ok(value) => {
                    queue!(
                        stdout,
                        cursor::MoveTo(0, *row),
                        SetForegroundColor(palette.accent),
                        SetBackgroundColor(palette.bg),
                        Print("  = "),
                        SetForegroundColor(palette.fg),
                        Print(fit(&value.to_string(), (cols as usize).saturating_sub(4))),
                        ResetColor,
                    )?;
                }

                Err(_) => {
                    queue!(
                        stdout,
                        cursor::MoveTo(0, *row),
                        SetForegroundColor(palette.dim),
                        SetBackgroundColor(palette.bg),
                        Print("  …"),
                        ResetColor,
                    )?;
                }
            }
        }

        *row += 2;
    }

    if state.math_history.is_empty() {
        queue!(
            stdout,
            cursor::MoveTo(0, *row),
            SetForegroundColor(palette.dim),
            SetBackgroundColor(palette.bg),
            Print("  no calculations yet"),
            ResetColor,
        )?;

        *row += 1;

        return Ok(());
    }

    if state.history_selected >= state.math_history.len() {
        state.history_selected = state.math_history.len() - 1;
    }

    let items: Vec<ListRow> = state
        .math_history
        .iter()
        .rev()
        .enumerate()
        .map(|(index, entry)| ListRow {
            text: format!("{}. {}", index + 1, entry),
            badge: None,
            color: palette.fg,
        })
        .collect();

    draw_list(
        stdout,
        row,
        term_rows,
        cols,
        bottom_lines,
        palette,
        &items,
        Some(state.history_selected),
    )
}

fn draw_settings_mode(
    state: &State,
    stdout: &mut io::Stdout,
    row: &mut u16,
    term_rows: u16,
    cols: u16,
    bottom_lines: u16,
    palette: &Palette,
) -> io::Result<()> {
    let items = vec![
        ListRow {
            text: format!("1. {:<14} {}", "background", state.settings.background),
            badge: None,
            color: palette.fg,
        },
        ListRow {
            text: format!("2. {:<14} {}", "footer(hints)", state.settings.footer),
            badge: None,
            color: palette.fg,
        },
        ListRow {
            text: format!("3. {:<14} {}", "timers", state.settings.timers),
            badge: None,
            color: palette.fg,
        },
        ListRow {
            text: format!("4. {:<14} {}", "display", state.settings.display.as_str()),
            badge: None,
            color: palette.fg,
        },
    ];

    draw_list(
        stdout,
        row,
        term_rows,
        cols,
        bottom_lines,
        palette,
        &items,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn draw_app_results(
    state: &mut State,
    stdout: &mut io::Stdout,
    matches: &[App],
    row: &mut u16,
    term_rows: u16,
    cols: u16,
    bottom_lines: u16,
    palette: &Palette,
) -> io::Result<()> {
    if matches.is_empty() {
        let message =
            if state.query.trim().is_empty() && state.settings.display == DisplayMode::None {
                "  type to search"
            } else {
                "  no matching apps"
            };

        queue!(
            stdout,
            cursor::MoveTo(0, *row),
            SetForegroundColor(palette.dim),
            SetBackgroundColor(palette.bg),
            Print(message),
            ResetColor,
        )?;

        *row += 1;

        return Ok(());
    }

    if state.selected >= matches.len() {
        state.selected = matches.len() - 1;
    }

    let items: Vec<ListRow> = matches
        .iter()
        .map(|app| ListRow {
            text: app.name.clone(),
            badge: if !app.actions.is_empty() {
                Some("›".to_string())
            } else {
                None
            },
            color: palette.fg,
        })
        .collect();

    draw_list(
        stdout,
        row,
        term_rows,
        cols,
        bottom_lines,
        palette,
        &items,
        Some(state.selected),
    )
}
