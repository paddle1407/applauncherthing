use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct Timer {
    pub label: String,
    pub end_unix: u64,
    pub finished: bool,
}

impl Timer {
    pub fn remaining(&self) -> Duration {
        let now = now_unix();

        if now >= self.end_unix {
            Duration::ZERO
        } else {
            Duration::from_secs(self.end_unix - now)
        }
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

pub type SharedTimers = Arc<Mutex<Vec<Timer>>>;

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

fn timers_file() -> PathBuf {
    data_dir().join("timers.txt")
}

fn parse_file(path: &Path) -> Vec<Timer> {
    let mut timers = Vec::new();

    if let Ok(contents) = fs::read_to_string(path) {
        for line in contents.lines() {
            let Some((end_str, label)) = line.split_once('\t') else {
                continue;
            };

            let Ok(end_unix) = end_str.parse::<u64>() else {
                continue;
            };

            let finished = now_unix() >= end_unix;
            timers.push(Timer {
                label: label.to_string(),
                end_unix,
                finished,
            });
        }
    }

    timers
}

pub fn load_shared() -> SharedTimers {
    Arc::new(Mutex::new(parse_file(&timers_file())))
}

#[cfg(unix)]
struct FileLock(File);

#[cfg(unix)]
impl FileLock {
    fn acquire(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;

        use std::os::fd::AsRawFd;
        let result = unsafe { flock(file.as_raw_fd(), LOCK_EX) };
        if result == -1 {
            return Err(io::Error::last_os_error());
        }

        Ok(Self(file))
    }
}

#[cfg(unix)]
impl Drop for FileLock {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        unsafe {
            flock(self.0.as_raw_fd(), LOCK_UN);
        }
    }
}

#[cfg(unix)]
const LOCK_EX: i32 = 2;
#[cfg(unix)]
const LOCK_UN: i32 = 8;

#[cfg(unix)]
unsafe extern "C" {
    fn flock(fd: i32, operation: i32) -> i32;
}

fn write_file(list: &[Timer]) -> io::Result<()> {
    let dir = data_dir();
    fs::create_dir_all(&dir)?;

    let path = timers_file();
    let temporary = dir.join(format!("timers.txt.{}.tmp", std::process::id()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)?;

        for timer in list {
            if timer.label.contains(['\t', '\r', '\n']) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "timer labels must be single-line",
                ));
            }
            writeln!(file, "{}\t{}", timer.end_unix, timer.label)?;
        }
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(unix)]
fn with_lock<T>(operation: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
    let dir = data_dir();
    fs::create_dir_all(&dir)?;
    let _lock = FileLock::acquire(&dir.join("timers.lock"))?;
    operation()
}

pub fn add(timers: &SharedTimers, timer: Timer) -> io::Result<()> {
    with_lock(|| {
        let mut list = parse_file(&timers_file());
        list.push(timer);
        write_file(&list)?;
        *timers
            .lock()
            .map_err(|_| io::Error::other("timer lock poisoned"))? = list;
        Ok(())
    })
}

pub fn clear_command(timers: &SharedTimers, argument: &str) -> io::Result<()> {
    let argument = argument.trim();

    with_lock(|| {
        let mut list = parse_file(&timers_file());

        if argument.is_empty() || argument.eq_ignore_ascii_case("all") {
            list.clear();
        } else if argument.eq_ignore_ascii_case("done") {
            list.retain(|timer| !timer.finished);
        } else {
            let Ok(number) = argument.parse::<usize>() else {
                return Ok(());
            };
            if number == 0 || number > list.len() {
                return Ok(());
            }
            list.remove(list.len() - number);
        }

        write_file(&list)?;
        *timers
            .lock()
            .map_err(|_| io::Error::other("timer lock poisoned"))? = list;
        Ok(())
    })
}

pub fn spawn_ticker(timers: SharedTimers) {
    thread::spawn(move || {
        loop {
            thread::sleep(Duration::from_millis(250));

            let _ = with_lock(|| {
                let mut list = parse_file(&timers_file());
                let mut changed = false;

                for timer in &mut list {
                    if !timer.finished && timer.remaining() == Duration::ZERO {
                        timer.finished = true;
                        changed = true;
                    }
                }

                if changed {
                    write_file(&list)?;
                }

                *timers
                    .lock()
                    .map_err(|_| io::Error::other("timer lock poisoned"))? = list;
                Ok(())
            });
        }
    });
}

pub fn parse_timer_input(input: &str) -> Option<(Duration, String)> {
    let input = input.trim();
    let mut parts = input.splitn(2, char::is_whitespace);
    let duration_part = parts.next()?;
    let label = parts.next().unwrap_or("").trim().to_string();
    let mut total_secs: u64 = 0;
    let mut current_num = String::new();
    let mut any_unit = false;

    for ch in duration_part.chars() {
        if ch.is_ascii_digit() {
            current_num.push(ch);
        } else {
            let value: u64 = current_num.parse().ok()?;
            current_num.clear();
            let multiplier = match ch {
                'h' | 'H' => 3600,
                'm' | 'M' => 60,
                's' | 'S' => 1,
                _ => return None,
            };
            total_secs = total_secs.checked_add(value.checked_mul(multiplier)?)?;
            any_unit = true;
        }
    }

    if !current_num.is_empty() {
        total_secs = total_secs.checked_add(current_num.parse::<u64>().ok()?)?;
        any_unit = true;
    }

    if !any_unit || total_secs == 0 {
        return None;
    }

    let label = if label.is_empty() {
        duration_part.to_string()
    } else {
        label
    };

    Some((Duration::from_secs(total_secs), label))
}

pub fn format_remaining(duration: Duration) -> String {
    let total = duration.as_secs();
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;

    if hours > 0 {
        format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
    } else {
        format!("{:02}:{:02}", minutes, seconds)
    }
}

#[cfg(test)]
mod tests {
    use super::{format_remaining, parse_timer_input};
    use std::time::Duration;

    #[test]
    fn parses_whitespace_separated_labels() {
        assert_eq!(
            parse_timer_input("5m\tcoffee"),
            Some((Duration::from_secs(300), "coffee".to_string()))
        );
    }

    #[test]
    fn rejects_duration_overflow() {
        assert!(parse_timer_input("18446744073709551615s").is_some());
        assert!(parse_timer_input("9999999999999999h").is_none());
    }

    #[test]
    fn formats_remaining() {
        assert_eq!(format_remaining(Duration::from_secs(3723)), "01:02:03");
        assert_eq!(format_remaining(Duration::from_secs(59)), "00:59");
    }
}
