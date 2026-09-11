//! Adding an account: `tailscale login`, watched from start to finish.
//!
//! The CLI owns the flow. It moves the daemon onto a new, empty account and
//! authenticates it one of two ways:
//!
//! - **in the browser**: it asks the control server for a login link, prints
//!   it, and waits until someone has used it. On Headscale that page usually
//!   ends in a command an admin has to run on the server;
//! - **with a pre-auth key**, which the server accepts on its own, so nobody
//!   has to visit anything.
//!
//! This module runs that process, lifts the link out of its output for the
//! window (and the browser), and cleans up when the login does not happen: a
//! login abandoned half way leaves the daemon on the empty account, and the
//! user disconnected from the one they were on, until something switches back.
//!
//! A login for an account already on this machine does not add a second one:
//! the daemon folds it into the existing account. That is what makes a key a
//! way back into an account whose node key expired.

use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{mpsc, oneshot};

use crate::actions::{self, Failure};
use crate::tailscale;
use crate::{i18n, store};

/// How long a login link is waited on. The CLI is given the same deadline, so
/// an elevated process this app cannot signal still ends on its own.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// The one login that can run at a time.
#[derive(Default)]
pub struct Jobs(Mutex<Option<Job>>);

struct Job {
    cancel: Option<oneshot::Sender<()>>,
    progress: Option<Progress>,
    with_key: bool,
}

/// What the window shows while the login waits.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub url: String,
    /// The link is the admin console's device approval, not a login: the
    /// account is in, and the tailnet wants an admin to let the device join.
    pub approval: bool,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Finished {
    pub ok: bool,
    pub cancelled: bool,
    pub error: Option<String>,
}

impl Jobs {
    pub fn running(&self) -> bool {
        self.0.lock().map(|job| job.is_some()).unwrap_or(false)
    }

    /// Whether the running login uses a pre-auth key, so a window opening in
    /// the middle of it knows there is no link to wait for.
    pub fn with_key(&self) -> bool {
        self.0
            .lock()
            .map(|job| job.as_ref().is_some_and(|j| j.with_key))
            .unwrap_or(false)
    }

    /// For a window that opens while a login is already waiting.
    pub fn progress(&self) -> Option<Progress> {
        self.0
            .lock()
            .ok()
            .and_then(|job| job.as_ref().and_then(|j| j.progress.clone()))
    }

    fn set_progress(&self, progress: Progress) {
        if let Ok(mut job) = self.0.lock() {
            if let Some(job) = job.as_mut() {
                job.progress = Some(progress);
            }
        }
    }

    pub fn cancel(&self) {
        if let Ok(mut job) = self.0.lock() {
            if let Some(cancel) = job.as_mut().and_then(|j| j.cancel.take()) {
                let _ = cancel.send(());
            }
        }
    }

    fn clear(&self) {
        if let Ok(mut job) = self.0.lock() {
            *job = None;
        }
    }
}

/// Start `tailscale login`, and return as soon as it is running.
///
/// With `auth_key` the account is authenticated by the key instead of a
/// browser login. Progress reaches the window as `login-progress` events and
/// the end as one `login-finished`.
pub async fn start(
    app: &AppHandle,
    server: Option<String>,
    nickname: Option<String>,
    auth_key: Option<String>,
) -> Result<(), Failure> {
    let server = tailscale::normalize_server(server.as_deref()).map_err(Failure::message)?;
    let nickname = nickname
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty());
    let auth_key = normalize_key(auth_key.as_deref()).map_err(Failure::message)?;
    let elevated = tailscale::needs_root_for_new_accounts();

    let mut args = vec!["login".to_string()];
    if let Some(url) = &server {
        args.push(format!("--login-server={url}"));
    }
    if let Some(nickname) = &nickname {
        args.push(format!("--nickname={nickname}"));
    }
    let key_file = match &auth_key {
        Some(key) => {
            let file = KeyFile::write(key).map_err(|e| Failure::message(e.to_string()))?;
            args.push(format!("--auth-key=file:{}", file.0.display()));
            Some(file)
        }
        None => None,
    };
    if elevated {
        // The account being created gets this user as its operator, so from
        // here on it can be switched to and from without root.
        let user =
            tailscale::local_user().ok_or_else(|| Failure::message(i18n::t("errors.no_user")))?;
        args.push(format!("--operator={user}"));
    }
    args.push(format!("--timeout={}s", LOGIN_TIMEOUT.as_secs()));

    let previous = crate::latest(app).active().map(|a| a.id.clone());

    let (cancel, cancelled) = oneshot::channel();
    {
        let jobs = app.state::<Jobs>();
        let mut slot = jobs
            .0
            .lock()
            .map_err(|_| Failure::message("login lock poisoned"))?;
        if slot.is_some() {
            return Err(Failure::message(i18n::t("errors.login_running")));
        }
        *slot = Some(Job {
            cancel: Some(cancel),
            progress: None,
            with_key: key_file.is_some(),
        });
    }

    let child = match tailscale::spawn(&args, elevated) {
        Ok(child) => child,
        Err(e) => {
            app.state::<Jobs>().clear();
            return Err(Failure::from(e));
        }
    };

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let finished = supervise(&app, child, cancelled, elevated).await;
        // The CLI read the key long before it got this far, or never will.
        drop(key_file);
        if finished.ok {
            if let Some(url) = &server {
                if let Err(e) = store::remember_server(url) {
                    eprintln!("remembering {url}: {e:#}");
                }
            }
        } else {
            restore(previous).await;
        }
        app.state::<Jobs>().clear();
        crate::refresh(&app).await;
        let _ = app.emit("login-finished", finished);
    });
    Ok(())
}

enum Outcome {
    Exited(Option<i32>, bool),
    Cancelled,
    TimedOut,
}

async fn supervise(
    app: &AppHandle,
    mut child: tokio::process::Child,
    cancelled: oneshot::Receiver<()>,
    elevated: bool,
) -> Finished {
    let (lines, mut incoming) = mpsc::unbounded_channel::<String>();
    if let Some(out) = child.stdout.take() {
        forward(out, lines.clone());
    }
    if let Some(err) = child.stderr.take() {
        forward(err, lines);
    }

    let mut transcript = Vec::new();
    let mut reader = LinkReader::default();
    let deadline = tokio::time::sleep(LOGIN_TIMEOUT + Duration::from_secs(30));
    tokio::pin!(deadline, cancelled);

    let outcome = loop {
        tokio::select! {
            Some(line) = incoming.recv() => {
                if let Some(progress) = reader.read(&line) {
                    announce(app, progress);
                }
                transcript.push(line);
            }
            status = child.wait() => {
                break match status {
                    Ok(status) => Outcome::Exited(status.code(), status.success()),
                    Err(_) => Outcome::Exited(None, false),
                };
            }
            _ = &mut cancelled => break Outcome::Cancelled,
            _ = &mut deadline => break Outcome::TimedOut,
        }
    };

    // Whatever the process managed to print before it went.
    while let Ok(Some(line)) =
        tokio::time::timeout(Duration::from_millis(200), incoming.recv()).await
    {
        transcript.push(line);
    }

    match outcome {
        Outcome::Exited(_, true) => Finished {
            ok: true,
            cancelled: false,
            error: None,
        },
        Outcome::Exited(code, false) if tailscale::is_elevation_refused(elevated, code) => {
            Finished {
                ok: false,
                cancelled: true,
                error: None,
            }
        }
        Outcome::Exited(_, false) => Finished {
            ok: false,
            cancelled: false,
            error: Some(tailscale::failure_text(&transcript.join("\n"))).filter(|e| !e.is_empty()),
        },
        Outcome::Cancelled => {
            // Root's process is not ours to signal; unelevated, it goes now.
            // An elevated one ends by itself once the switch back below puts
            // the daemon on a running account, or at its own timeout.
            let _ = child.start_kill();
            Finished {
                ok: false,
                cancelled: true,
                error: None,
            }
        }
        Outcome::TimedOut => {
            let _ = child.start_kill();
            Finished {
                ok: false,
                cancelled: false,
                error: Some(i18n::t("errors.login_timeout")),
            }
        }
    }
}

fn forward(
    stream: impl tokio::io::AsyncRead + Unpin + Send + 'static,
    lines: mpsc::UnboundedSender<String>,
) {
    tauri::async_runtime::spawn(async move {
        let mut reader = BufReader::new(stream).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            if lines.send(line).is_err() {
                break;
            }
        }
    });
}

fn announce(app: &AppHandle, progress: Progress) {
    let first = app.state::<Jobs>().progress().is_none();
    app.state::<Jobs>().set_progress(progress.clone());
    // Opened once, for the login itself: the user asked for it a moment ago.
    // The approval link is for an admin, and the window says so instead.
    if first && !progress.approval {
        if let Err(failure) = actions::open_url(&progress.url) {
            eprintln!("opening the login link: {}", failure.message);
        }
    }
    let _ = app.emit("login-progress", progress);
}

/// Put the daemon back on the account the login started from.
///
/// Usually a plain switch: an elevated login hands the new account this user as
/// its operator before it asks for a link. A login that failed before getting
/// that far leaves an empty account nobody but root may move off, and then the
/// switch back asks for the password too — the alternative is a user left
/// disconnected with no way back from here.
async fn restore(previous: Option<String>) {
    let Some(previous) = previous else { return };
    let current = tailscale::snapshot(&[]).await;
    if current.active().map(|a| a.id.as_str()) == Some(previous.as_str()) {
        return;
    }
    let failure = match actions::switch_to(&previous).await {
        Ok(()) => return,
        Err(failure) => failure,
    };
    if failure.access_denied && tailscale::can_elevate() {
        if let Err(failure) = actions::switch_as_root(&previous).await {
            eprintln!("switching back to {previous}: {}", failure.message);
        }
    } else {
        eprintln!("switching back to {previous}: {}", failure.message);
    }
}

/// A pre-auth key as pasted, or `None` for a browser login.
///
/// Keys are one token — `tskey-auth-…` from Tailscale, a hex string from
/// `headscale preauthkeys create` — so anything with a space or a line break
/// in it after trimming is a paste gone wrong, better refused here than sent to
/// a control server as a login attempt.
fn normalize_key(input: Option<&str>) -> Result<Option<String>, String> {
    let Some(key) = input.map(str::trim).filter(|k| !k.is_empty()) else {
        return Ok(None);
    };
    if key.chars().any(char::is_whitespace) || key.len() > 1024 {
        return Err(i18n::t("errors.invalid_key"));
    }
    Ok(Some(key.to_string()))
}

/// A pre-auth key handed to the CLI through a file rather than its command
/// line, where anyone on the machine could read it in the process list — the
/// CLI's `--auth-key=file:` form exists for exactly this.
///
/// Readable by this user only, in the per-user runtime directory (a tmpfs on
/// Linux) when there is one. Root, which runs the login on Linux, reads it all
/// the same. Removed when dropped.
struct KeyFile(PathBuf);

impl KeyFile {
    fn write(key: &str) -> std::io::Result<Self> {
        use std::io::Write;

        let dir = dirs::runtime_dir().unwrap_or_else(std::env::temp_dir);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = dir.join(format!(
            "tailscale-switch-key-{}-{nanos}",
            std::process::id()
        ));

        let mut options = std::fs::OpenOptions::new();
        // Never an existing file: a name someone else created first is not one
        // to write a key into.
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = KeyFile(path);
        options.open(&file.0)?.write_all(key.as_bytes())?;
        Ok(file)
    }
}

impl Drop for KeyFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Picks the links out of the CLI's output, line by line.
///
/// The login link comes on the line after "To authenticate, visit:", and the
/// device approval link after "To approve your machine". Other output can carry
/// links of its own — warnings point at tailscale.com/s/… pages — so a link
/// only counts after one of those two lines, or when it is shaped like a login
/// link on its own.
#[derive(Default)]
struct LinkReader {
    expecting: Option<bool>,
}

impl LinkReader {
    fn read(&mut self, line: &str) -> Option<Progress> {
        let lower = line.to_ascii_lowercase();
        if lower.contains("to authenticate, visit") {
            self.expecting = Some(false);
            return None;
        }
        if lower.contains("to approve your machine") {
            self.expecting = Some(true);
            return None;
        }

        let url = line
            .split_whitespace()
            .find(|word| word.starts_with("https://") || word.starts_with("http://"))?;
        let approval = match self.expecting.take() {
            Some(approval) => approval,
            None if url.contains("/a/") || url.contains("/register/") => false,
            None => return None,
        };
        Some(Progress {
            url: url.to_string(),
            approval,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_one_token() {
        assert_eq!(normalize_key(None), Ok(None));
        assert_eq!(normalize_key(Some("  \n")), Ok(None));
        assert_eq!(
            normalize_key(Some(" tskey-auth-kX1-abc\n")),
            Ok(Some("tskey-auth-kX1-abc".into()))
        );
        assert!(normalize_key(Some("abc def")).is_err());
        assert!(normalize_key(Some("abc\ndef")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn the_key_file_is_private_and_goes_away() {
        use std::os::unix::fs::PermissionsExt;

        let file = KeyFile::write("tskey-auth-test").unwrap();
        let path = file.0.clone();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "tskey-auth-test");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        drop(file);
        assert!(!path.exists());
    }

    #[test]
    fn finds_the_login_link() {
        let mut reader = LinkReader::default();
        assert_eq!(reader.read(""), None);
        assert_eq!(reader.read("To authenticate, visit:"), None);
        assert_eq!(reader.read(""), None);
        assert_eq!(
            reader.read("\thttps://headscale.example.com/register/nodekey:abc"),
            Some(Progress {
                url: "https://headscale.example.com/register/nodekey:abc".into(),
                approval: false
            })
        );
    }

    #[test]
    fn ignores_links_in_warnings() {
        let mut reader = LinkReader::default();
        assert_eq!(
            reader.read("Warning: UDP GRO forwarding is suboptimally configured, see https://tailscale.com/s/ethtool-config-udp-gro"),
            None
        );
    }

    #[test]
    fn a_login_shaped_link_counts_without_its_heading() {
        let mut reader = LinkReader::default();
        let progress = reader
            .read("https://login.tailscale.com/a/1234abcd")
            .unwrap();
        assert!(!progress.approval);
    }

    #[test]
    fn tells_approval_from_login() {
        let mut reader = LinkReader::default();
        reader.read("To approve your machine, visit (as admin):");
        let progress = reader
            .read("\thttps://login.tailscale.com/admin/machines")
            .unwrap();
        assert!(progress.approval);
    }
}
