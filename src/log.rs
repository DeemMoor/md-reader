//! Error log: panics, failed operations and page script errors go to a small text
//! file the user can open from the Help menu and attach to a bug report.

use std::fmt::Display;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// The log is rotated to `mdreader.old.log` when it grows past this.
const MAX_SIZE: u64 = 512 * 1024;
const ISSUES_URL: &str = "https://github.com/DeemMoor/md-reader/issues/new";
const MAX_URL: usize = 7500;

static LOCK: Mutex<()> = Mutex::new(());

pub fn path() -> Option<PathBuf> {
    Some(dirs::data_local_dir()?.join("com.deemmoor.mdreader").join("mdreader.log"))
}

/// Rotates the log, notes the start and installs the panic hook.
pub fn init() {
    if let Some(path) = path() {
        if fs::metadata(&path).is_ok_and(|m| m.len() > MAX_SIZE) {
            let _ = fs::rename(&path, path.with_extension("old.log"));
        }
    }
    write("INFO", format!("MD Reader {} started", env!("CARGO_PKG_VERSION")));

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let location = info.location().map(|l| format!(" at {}:{}", l.file(), l.line())).unwrap_or_default();
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown error".into());
        write("CRASH", format!("thread '{}'{location}: {message}", thread.name().unwrap_or("?")));
        default_hook(info);
        // Only a panic on the UI thread takes the app down; others are just logged.
        if thread.name() != Some("main") {
            return;
        }
        // The UI may be gone already; a native dialog still works.
        let report = rfd::MessageDialog::new()
            .set_title("MD Reader")
            .set_description(
                "В программе произошла ошибка, и она будет закрыта. Сведения записаны в журнал.\n\n\
                 Открыть страницу отчёта об ошибке на GitHub?",
            )
            .set_level(rfd::MessageLevel::Error)
            .set_buttons(rfd::MessageButtons::YesNo)
            .show();
        if report == rfd::MessageDialogResult::Yes {
            let _ = open::that_detached(issue_url());
        }
    }));
}

pub fn error(message: impl Display) {
    write("ERROR", message);
}

fn write(level: &str, message: impl Display) {
    let Some(path) = path() else { return };
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(file, "{} {level} {message}", timestamp(SystemTime::now()));
    }
}

/// "YYYY-MM-DD HH:MM:SSZ" in UTC.
fn timestamp(time: SystemTime) -> String {
    let secs = time.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (days, rem) = (secs / 86400, secs % 86400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days as i64 + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// The last `n` lines of the log.
fn tail(n: usize) -> String {
    let text = path().and_then(|p| fs::read_to_string(p).ok()).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

/// A prefilled "new issue" page: system facts and the end of the log.
pub fn issue_url() -> String {
    let about = crate::server::about_text();
    let mut log = tail(40);
    loop {
        let body = format!(
            "### Что случилось\n\n\n\n### Как повторить\n\n1. \n\n### Сведения\n\n```\n{about}\n```\n\n\
             ### Журнал\n\n<!-- Проверьте, что здесь нет личных данных, например путей к файлам. -->\n\n```\n{log}\n```\n"
        );
        let url = format!("{ISSUES_URL}?body={}", encode(&body));
        // GitHub rejects much longer prefilled URLs; drop the oldest log lines until it fits.
        if url.len() <= MAX_URL || log.is_empty() {
            return url;
        }
        log = log.split_once('\n').map(|(_, rest)| rest.to_string()).unwrap_or_default();
    }
}

fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn timestamps_are_utc_dates() {
        assert_eq!(timestamp(UNIX_EPOCH), "1970-01-01 00:00:00Z");
        assert_eq!(timestamp(UNIX_EPOCH + Duration::from_secs(951_782_400)), "2000-02-29 00:00:00Z");
        assert_eq!(timestamp(UNIX_EPOCH + Duration::from_secs(1_792_108_799)), "2026-10-15 23:59:59Z");
    }

    #[test]
    fn issue_url_stays_short() {
        let url = issue_url();
        assert!(url.starts_with(ISSUES_URL));
        assert!(url.len() <= MAX_URL, "{}", url.len());
    }

    #[test]
    fn urls_are_percent_encoded() {
        assert_eq!(encode("a b/ё#"), "a%20b%2F%D1%91%23");
    }
}
