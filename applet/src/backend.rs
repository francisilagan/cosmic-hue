//! The link to the Python backend: one long-lived `python3 <backend> serve`
//! process that speaks newline-delimited JSON (see docs/PROTOCOL.md).
//!
//! The applet never talks to the bridge itself. This module starts the
//! backend, forwards its events as [`Event::Line`], and writes the commands it
//! receives on the channel from [`Event::Started`]. If the process stops, it is
//! started again after a short delay.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use cosmic::iced::futures::channel::mpsc as futures_mpsc;
use cosmic::iced::futures::{SinkExt, Stream};
use cosmic::iced::stream;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

const RESTART_DELAY: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub enum Event {
    /// The backend runs. Commands sent on this channel go to its stdin.
    Started(mpsc::UnboundedSender<String>),
    /// One event object from the backend.
    Line(serde_json::Value),
    /// The backend stopped or could not start, with the reason.
    Stopped(String),
}

fn is_backend(dir: &Path) -> bool {
    dir.join("__main__.py").is_file() && dir.join("hue_backend").is_dir()
}

/// Where the backend package is installed.
///
/// `COSMIC_HUE_BACKEND` wins; then the user's data dir; then the system dirs.
pub fn locate() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("COSMIC_HUE_BACKEND").map(PathBuf::from) {
        return is_backend(&dir).then_some(dir);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| home.map(|h| h.join(".local/share")));
    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    data_home
        .into_iter()
        .chain(data_dirs.split(':').map(PathBuf::from))
        .map(|d| d.join("cosmic-hue/backend"))
        .find(|d| is_backend(d))
}

/// The stream behind the applet's subscription.
pub fn run() -> impl Stream<Item = Event> {
    stream::channel(64, async move |mut out: futures_mpsc::Sender<Event>| {
        loop {
            let reason = match locate() {
                Some(dir) => session(&dir, &mut out).await,
                None => "backend not found".to_string(),
            };
            if out.send(Event::Stopped(reason)).await.is_err() {
                return;
            }
            tokio::time::sleep(RESTART_DELAY).await;
        }
    })
}

/// Run one backend process until it exits. Returns why it stopped.
async fn session(dir: &Path, out: &mut futures_mpsc::Sender<Event>) -> String {
    let mut child = match Command::new("python3")
        .arg(dir)
        .arg("serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => child,
        Err(err) => return format!("cannot start python3: {err}"),
    };
    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        return "backend pipes missing".to_string();
    };
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    if out.send(Event::Started(tx)).await.is_err() {
        return "applet closed".to_string();
    }
    let mut lines = BufReader::new(stdout).lines();
    loop {
        tokio::select! {
            line = lines.next_line() => match line {
                Ok(Some(line)) => {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line)
                        && out.send(Event::Line(value)).await.is_err() {
                            return "applet closed".to_string();
                        }
                }
                Ok(None) | Err(_) => break,
            },
            cmd = rx.recv() => match cmd {
                Some(mut cmd) => {
                    cmd.push('\n');
                    if stdin.write_all(cmd.as_bytes()).await.is_err() || stdin.flush().await.is_err() {
                        break;
                    }
                }
                // Every sender was dropped: the applet replaced this session.
                None => break,
            },
        }
    }
    let _ = child.kill().await;
    "backend stopped".to_string()
}
