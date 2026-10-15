//! Single instance: a second launch hands its files to the running window and exits.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::thread;

use interprocess::local_socket::{GenericNamespaced, ListenerOptions, Stream, prelude::*};

fn socket_name() -> String {
    let user = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_default();
    format!("mdreader-{user}.sock")
}

/// Sends `paths` to an already running instance. Returns true if one received them.
pub fn forward_to_running(paths: &[PathBuf]) -> bool {
    let Ok(name) = socket_name().to_ns_name::<GenericNamespaced>() else { return false };
    let Ok(mut stream) = Stream::connect(name) else { return false };
    let mut msg = String::new();
    for path in paths {
        msg.push_str(&path.to_string_lossy());
        msg.push('\n');
    }
    // An empty message still means "bring the window to front".
    msg.push('\n');
    stream.write_all(msg.as_bytes()).is_ok()
}

/// Becomes the primary instance; `on_open` receives the files of later launches.
pub fn listen(on_open: impl Fn(Vec<PathBuf>) + Send + 'static) {
    let Ok(name) = socket_name().to_ns_name::<GenericNamespaced>() else { return };
    let Ok(listener) = ListenerOptions::new().name(name).create_sync() else { return };
    thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            let mut paths = Vec::new();
            for line in BufReader::new(conn).lines() {
                let Ok(line) = line else { break };
                if line.is_empty() {
                    break;
                }
                paths.push(PathBuf::from(line));
            }
            on_open(paths);
        }
    });
}
