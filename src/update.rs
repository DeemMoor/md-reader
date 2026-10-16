//! Update check against the latest GitHub release and, on Windows, a one-click
//! install: download the NSIS installer, start it and quit.

use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

const LATEST_URL: &str = "https://api.github.com/repos/DeemMoor/md-reader/releases/latest";
const USER_AGENT: &str = concat!("MD-Reader/", env!("CARGO_PKG_VERSION"));
/// Published with every release; an installer is only run if its hash is listed there.
const CHECKSUMS: &str = "SHA256SUMS.txt";

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    pub version: String,
    pub newer: bool,
    /// Markdown from the release description.
    pub notes: String,
    pub page: String,
    /// Installer that can update this install in place (Windows only for now).
    pub installer: Option<String>,
    pub checksums: Option<String>,
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30))
        .user_agent(USER_AGENT)
        .build()
}

pub fn check() -> Result<Release, String> {
    let body = agent()
        .get(LATEST_URL)
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(describe)?
        .into_string()
        .map_err(|e| e.to_string())?;
    let json: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let tag = json["tag_name"].as_str().unwrap_or_default();
    let version = tag.trim_start_matches('v').to_string();
    let asset = |matches: &dyn Fn(&str) -> bool| {
        json["assets"].as_array().and_then(|assets| {
            assets
                .iter()
                .find(|a| a["name"].as_str().is_some_and(matches))
                .and_then(|a| a["browser_download_url"].as_str())
                .map(String::from)
        })
    };
    let installer = if cfg!(windows) { asset(&|n| n.ends_with("-setup.exe")) } else { None };
    let checksums = asset(&|n| n == CHECKSUMS);
    Ok(Release {
        newer: is_newer(&version, env!("CARGO_PKG_VERSION")),
        version,
        notes: json["body"].as_str().unwrap_or_default().to_string(),
        page: json["html_url"].as_str().unwrap_or_default().to_string(),
        installer,
        checksums,
    })
}

fn describe(err: ureq::Error) -> String {
    match err {
        ureq::Error::Status(404, _) => "релизов пока нет".into(),
        ureq::Error::Status(code, _) => format!("сервер ответил {code}"),
        ureq::Error::Transport(t) => format!("нет связи с GitHub ({})", t.kind()),
    }
}

fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let mut parts = v.split(['.', '-']).map(|p| p.parse::<u64>().ok());
    Some((parts.next()??, parts.next()??, parts.next().flatten().unwrap_or(0)))
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

/// The hash listed for `file` in a `sha256sum` listing.
fn expected_hash(listing: &str, file: &str) -> Option<String> {
    listing.lines().find_map(|line| {
        let (hash, name) = line.trim().split_once(char::is_whitespace)?;
        (name.trim().trim_start_matches('*') == file).then(|| hash.to_ascii_lowercase())
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Downloads the installer at `url` into the temp dir, reporting progress in percent,
/// and checks it against the release's checksum listing at `checksums`.
pub fn download(url: &str, checksums: &str, mut progress: impl FnMut(u32)) -> Result<PathBuf, String> {
    let name = url.rsplit('/').next().filter(|n| !n.is_empty()).unwrap_or("mdreader-setup.exe");
    if checksums.is_empty() {
        return Err("в релизе нет контрольных сумм, скачайте установщик со страницы релиза".into());
    }
    let listing = agent().get(checksums).call().map_err(describe)?.into_string().map_err(|e| e.to_string())?;
    let expected = expected_hash(&listing, name).ok_or("установщика нет в списке контрольных сумм")?;
    let dir = std::env::temp_dir().join("mdreader-update");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(name);

    let response = agent().get(url).call().map_err(describe)?;
    let total: u64 = response.header("Content-Length").and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut reader = response.into_reader();
    let mut file = fs::File::create(&path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut done: u64 = 0;
    let mut last = u32::MAX;
    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        hasher.update(&buf[..n]);
        done += n as u64;
        if total > 0 {
            let pct = (done * 100 / total) as u32;
            if pct != last {
                last = pct;
                progress(pct);
            }
        }
    }
    drop(file);
    if total > 0 && done != total {
        let _ = fs::remove_file(&path);
        return Err("загрузка оборвалась".into());
    }
    if hex(&hasher.finalize()) != expected {
        let _ = fs::remove_file(&path);
        return Err("контрольная сумма не совпала, файл повреждён или подменён".into());
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("0.3.0", "0.2.2"));
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("1.0", "0.9.9"));
        assert!(!is_newer("0.2.2", "0.2.2"));
        assert!(!is_newer("0.2.1", "0.2.2"));
        assert!(!is_newer("garbage", "0.2.2"));
    }

    #[test]
    fn checksum_listing_is_parsed() {
        let listing = "ABC123  mdreader_0.3.0_x64-setup.exe
                       def456 *mdreader_0.3.0_x64_ru-RU.msi
";
        assert_eq!(expected_hash(listing, "mdreader_0.3.0_x64-setup.exe").as_deref(), Some("abc123"));
        assert_eq!(expected_hash(listing, "mdreader_0.3.0_x64_ru-RU.msi").as_deref(), Some("def456"));
        assert_eq!(expected_hash(listing, "other.exe"), None);
    }

    #[test]
    fn sha256_hex() {
        assert_eq!(
            hex(&Sha256::digest(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
