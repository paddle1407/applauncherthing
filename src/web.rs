use std::io;
use std::process::Command;

pub fn search_url(query: &str) -> String {
    let query = query.trim();
    if query.starts_with("http://") || query.starts_with("https://") {
        return query.to_string();
    }
    format!("https://duckduckgo.com/?q={}", url_encode(query))
}

pub fn open_web(query: &str) -> io::Result<()> {
    let url = search_url(query);
    Command::new("xdg-open")
        .arg(&url)
        .spawn()?
        .wait()
        .map(|_| ())
}

fn url_encode(s: &str) -> String {
    let mut encoded = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(b as char);
            }
            b' ' => encoded.push('+'),
            _ => {
                encoded.push_str(&format!("%{:02X}", b));
            }
        }
    }
    encoded
}
