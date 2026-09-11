//! The changes the window and the tray can ask for, shared by both.

use serde::Serialize;
use std::time::Duration;

use crate::i18n;
use crate::tailscale;

/// Switching waits for the new account to come up; a slow control server can
/// take a while, but not this long.
const CHANGE_TIMEOUT: Duration = Duration::from_secs(45);

/// A root prompt waits on a person typing a password.
const ELEVATED_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// A change that did not go through, as the window needs to hear it.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Failure {
    pub message: String,
    /// The daemon refused this user on the account in front. Granting access
    /// — once per account — is the fix, and the window offers it.
    pub access_denied: bool,
    /// The password prompt was dismissed. An answer rather than an error.
    pub cancelled: bool,
}

impl Failure {
    pub fn message(message: impl Into<String>) -> Self {
        let message = message.into();
        Failure {
            access_denied: tailscale::is_access_denied(&message),
            cancelled: false,
            message,
        }
    }

    fn cancelled() -> Self {
        Failure {
            message: i18n::t("errors.cancelled"),
            access_denied: false,
            cancelled: true,
        }
    }
}

impl From<anyhow::Error> for Failure {
    fn from(e: anyhow::Error) -> Self {
        Failure::message(format!("{e:#}"))
    }
}

impl From<crate::localapi::Error> for Failure {
    fn from(e: crate::localapi::Error) -> Self {
        let denied = matches!(e, crate::localapi::Error::Denied(_));
        let mut failure = Failure::message(e.to_string());
        failure.access_denied |= denied;
        failure
    }
}

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|a| a.to_string()).collect()
}

/// Run a change and turn whatever the CLI said into a `Failure`.
async fn change(list: &[&str]) -> Result<tailscale::Run, Failure> {
    let run = tailscale::run(&args(list), false, CHANGE_TIMEOUT).await?;
    if run.ok {
        Ok(run)
    } else {
        Err(Failure::message(tailscale::failure_text(&run.text())))
    }
}

pub async fn switch_to(id: &str) -> Result<(), Failure> {
    let run = tailscale::run(&args(&["switch", id]), false, CHANGE_TIMEOUT).await?;
    // The CLI moves the daemon, then waits for the account to come up and
    // exits 1 if it does not — "Tailscale is stopped." for an account that was
    // turned off. The switch itself happened all the same, and it said so first.
    if run.ok || run.stdout.contains("Switching to account") {
        return Ok(());
    }
    Err(Failure::message(tailscale::failure_text(&run.text())))
}

/// The same switch, as root: the way back from an account no one else may
/// move off.
pub async fn switch_as_root(id: &str) -> Result<(), Failure> {
    let run = tailscale::run(&args(&["switch", id]), true, ELEVATED_TIMEOUT).await?;
    if tailscale::is_elevation_refused(true, run.code) {
        return Err(Failure::cancelled());
    }
    if run.ok || run.stdout.contains("Switching to account") {
        return Ok(());
    }
    Err(Failure::message(tailscale::failure_text(&run.text())))
}

/// Take an account off this machine. Not the one in front: the CLI quietly
/// does nothing with that, so it is refused here with a reason instead.
pub async fn remove(id: &str, active: Option<&str>) -> Result<(), Failure> {
    if active == Some(id) {
        return Err(Failure::message(i18n::t("errors.active_account")));
    }
    change(&["switch", "remove", id]).await.map(|_| ())
}

/// Log the account in front out: the node key is expired on the control
/// server and the account leaves this machine.
pub async fn logout() -> Result<(), Failure> {
    change(&["logout"]).await.map(|_| ())
}

pub async fn connect() -> Result<(), Failure> {
    change(&["up"]).await.map(|_| ())
}

pub async fn disconnect() -> Result<(), Failure> {
    change(&["down"]).await.map(|_| ())
}

/// Set the nickname of the account in front; empty gives the login name back.
pub async fn rename(name: &str) -> Result<(), Failure> {
    let flag = format!("--nickname={}", name.trim());
    change(&["set", &flag]).await.map(|_| ())
}

/// Make this user the operator of the account in front, through the system's
/// password prompt. Once per account: the setting is kept with it.
pub async fn grant_access() -> Result<(), Failure> {
    let user =
        tailscale::local_user().ok_or_else(|| Failure::message(i18n::t("errors.no_user")))?;
    let flag = format!("--operator={user}");
    let run = tailscale::run(&args(&["set", &flag]), true, ELEVATED_TIMEOUT).await?;
    if tailscale::is_elevation_refused(true, run.code) {
        return Err(Failure::cancelled());
    }
    if !run.ok {
        return Err(Failure::message(tailscale::failure_text(&run.text())));
    }
    Ok(())
}

/// Start a login on the account in front and open its link.
///
/// Returns the link, or `None` when the account turned out not to need one
/// (the daemon went straight back to running).
pub async fn reauthenticate() -> Result<Option<String>, Failure> {
    if !tailscale::start_interactive_login().await? {
        // No socket to ask: the CLI's `up` starts the login instead. It waits
        // for the login to finish, so it is left running in the background;
        // the link turns up in the status either way.
        let child = tailscale::spawn(&args(&["up", &format!("--timeout={}s", 10 * 60)]), false)?;
        tauri::async_runtime::spawn(async move {
            let _ = child.wait_with_output().await;
        });
    }

    // The link comes from the control server, a moment after the request.
    for _ in 0..30 {
        if let Some(url) = tailscale::auth_url().await {
            open_url(&url)?;
            return Ok(Some(url));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Ok(None)
}

/// Hand a web link to the desktop's browser.
pub fn open_url(url: &str) -> Result<(), Failure> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(Failure::message(format!("not a web link: {url}")));
    }
    #[cfg(target_os = "macos")]
    let opener = "open";
    #[cfg(not(target_os = "macos"))]
    let opener = "xdg-open";

    let mut child = std::process::Command::new(opener)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| {
            Failure::message(i18n::t_args(
                "errors.no_opener",
                &[("error", &e.to_string())],
            ))
        })?;
    // Reaped off to the side, or every link opened leaves a zombie behind.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}
