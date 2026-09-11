//! What Tailscale says about this computer, and the CLI that changes it.
//!
//! Tailscale already keeps several accounts side by side — "fast user
//! switching", a profile per login, held by the daemon. So unlike its Claude
//! sibling this app stores no credentials of its own: it reads the daemon and
//! drives the `tailscale` CLI.
//!
//! Reads go to the LocalAPI socket first and fall back to the CLI's JSON
//! output (see `localapi`). Changes always go through the CLI: it is the
//! documented surface, it already knows how to wait for a switch to settle, and
//! it is the one thing that exists on every platform.
//!
//! One rule of the daemon shapes a good part of this app. On Linux a non-root
//! user may change anything only if they are the *operator* of the account in
//! front — and the operator is a preference of each account, not of the
//! machine. An account created by `tailscale login` starts with none, so the
//! same user who could create it cannot drive it afterwards without root. See
//! `needs_root_for_new_accounts` and the login flow in `login.rs`.

use anyhow::{anyhow, Result};
use serde::Serialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Mutex;
use std::time::Duration;
use tokio::process::{Child, Command};

use crate::i18n;
use crate::localapi::{self, Error as ApiError};

/// A read of the CLI: a local process against a local daemon.
const READ_TIMEOUT: Duration = Duration::from_secs(8);

/// Where the daemon is in its life, plus the two states that come before
/// there is a daemon to ask.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DaemonState {
    NotInstalled,
    DaemonDown,
    NoState,
    NeedsLogin,
    NeedsMachineAuth,
    Stopped,
    Starting,
    Running,
}

impl DaemonState {
    fn from_backend(name: &str) -> DaemonState {
        match name {
            "NeedsLogin" => DaemonState::NeedsLogin,
            "NeedsMachineAuth" => DaemonState::NeedsMachineAuth,
            "Stopped" => DaemonState::Stopped,
            "Starting" => DaemonState::Starting,
            "Running" => DaemonState::Running,
            _ => DaemonState::NoState,
        }
    }
}

/// One login profile on this machine.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    /// Tailscale's own name for the profile: the nickname when one is set, the
    /// login name otherwise.
    pub name: String,
    pub login_name: Option<String>,
    pub display_name: Option<String>,
    pub tailnet: Option<String>,
    /// Only the socket reports it for every account; over the CLI it is known
    /// for the active one and remembered by the store from then on.
    pub control_url: Option<String>,
    pub active: bool,
}

/// This device, as the account in front sees it.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub state: DaemonState,
    pub version: Option<String>,
    pub hostname: Option<String>,
    pub dns_name: Option<String>,
    pub ipv4: Option<String>,
    pub ipv6: Option<String>,
    pub tailnet: Option<String>,
    pub login_name: Option<String>,
    pub exit_node: Option<String>,
    /// RFC 3339, when the tailnet expires node keys at all.
    pub key_expiry: Option<String>,
    /// Set while a login is waiting for someone to open it.
    pub auth_url: Option<String>,
}

impl Connection {
    fn empty(state: DaemonState) -> Self {
        Connection {
            state,
            version: None,
            hostname: None,
            dns_name: None,
            ipv4: None,
            ipv6: None,
            tailnet: None,
            login_name: None,
            exit_node: None,
            key_expiry: None,
            auth_url: None,
        }
    }
}

/// Everything the window and the tray draw, read in one pass.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub connection: Connection,
    pub accounts: Vec<Account>,
    /// False when the daemon refuses this user changes on the account in front.
    pub can_manage: bool,
    /// The daemon's own words, when it could not be read at all.
    pub problem: Option<String>,
    /// Access can be granted from here, through the system's password prompt.
    pub can_elevate: bool,
    /// Adding an account will bring up that prompt.
    pub login_needs_root: bool,
    /// The local user, for the command that grants access by hand.
    pub user: Option<String>,
    /// The account list is the daemon's own, just read — not one kept from
    /// before because the daemon would not give one.
    #[serde(skip)]
    pub complete: bool,
}

impl Snapshot {
    pub fn unavailable(state: DaemonState, problem: Option<String>) -> Self {
        Snapshot {
            connection: Connection::empty(state),
            accounts: Vec::new(),
            can_manage: true,
            problem,
            can_elevate: can_elevate(),
            login_needs_root: needs_root_for_new_accounts(),
            user: local_user(),
            complete: false,
        }
    }

    pub fn active(&self) -> Option<&Account> {
        self.accounts.iter().find(|a| a.active)
    }
}

/// The parts of the active account's preferences this app looks at.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Prefs {
    pub control_url: Option<String>,
}

// ---------------------------------------------------------------------------
// Finding things
// ---------------------------------------------------------------------------

/// The `tailscale` binary. GUI sessions often start apps with a thinner PATH
/// than a terminal has, so the usual install locations are tried after it.
pub fn binary() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("TAILSCALE_SWITCH_CLI") {
        return Some(PathBuf::from(path));
    }
    if let Some(path) = which("tailscale") {
        return Some(path);
    }
    [
        "/usr/bin/tailscale",
        "/usr/local/bin/tailscale",
        "/usr/sbin/tailscale",
        "/snap/bin/tailscale",
        "/opt/homebrew/bin/tailscale",
        "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|path| path.is_file())
}

pub fn which(bin: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .map(|dir| dir.join(bin))
        .find(|path| path.is_file())
}

/// polkit's command runner: the system draws the password prompt and keeps the
/// password, and this app only ever sees whether it was given.
#[cfg(target_os = "linux")]
fn pkexec() -> Option<PathBuf> {
    which("pkexec").or_else(|| Some(PathBuf::from("/usr/bin/pkexec")).filter(|p| p.is_file()))
}

pub fn can_elevate() -> bool {
    #[cfg(target_os = "linux")]
    {
        pkexec().is_some()
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// The name of the user running this app, as `--operator` wants it.
pub fn local_user() -> Option<String> {
    let from_env = ["USER", "LOGNAME"]
        .iter()
        .filter_map(|var| std::env::var(var).ok())
        .find(|name| !name.is_empty());
    if from_env.is_some() {
        return from_env;
    }
    #[cfg(unix)]
    {
        // SAFETY: getpwuid returns a pointer into static storage or null; the
        // name is copied out before anything else can call it again.
        unsafe {
            let entry = libc::getpwuid(libc::getuid());
            if !entry.is_null() && !(*entry).pw_name.is_null() {
                let name = std::ffi::CStr::from_ptr((*entry).pw_name);
                return Some(name.to_string_lossy().into_owned());
            }
        }
    }
    None
}

/// Whether creating an account has to run as root.
///
/// On Linux the daemon runs as root and gates changes on the operator of the
/// account in front. `tailscale login` starts by moving to a brand-new account,
/// which has no operator — so a login run as the user stops half way, on an
/// empty account the user can no longer drive, with the old one disconnected.
/// Run as root it can set the operator on the new account as it creates it.
///
/// Not needed when this app already is root, or when the daemon runs as this
/// very user (userspace networking), which it always trusts.
pub fn needs_root_for_new_accounts() -> bool {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::MetadataExt;
        // SAFETY: getuid cannot fail.
        let uid = unsafe { libc::getuid() };
        if uid == 0 {
            return false;
        }
        match localapi::socket().and_then(|socket| std::fs::metadata(socket).ok()) {
            Some(meta) => meta.uid() != uid,
            None => true,
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

// ---------------------------------------------------------------------------
// Running the CLI
// ---------------------------------------------------------------------------

pub struct Run {
    pub ok: bool,
    /// Exit code, when there is one.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Run {
    /// What the CLI said, stdout and stderr together: `tailscale switch` puts
    /// half of its story on each.
    pub fn text(&self) -> String {
        let mut text = self.stdout.trim().to_string();
        if !self.stderr.trim().is_empty() {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(self.stderr.trim());
        }
        text
    }
}

/// The command line for `tailscale <args>`, through pkexec when `elevated`.
fn command(args: &[String], elevated: bool) -> Result<Command> {
    let program = binary().ok_or_else(|| anyhow!(i18n::t("errors.not_installed")))?;
    let mut command = if elevated {
        #[cfg(target_os = "linux")]
        {
            let pkexec = pkexec().ok_or_else(|| anyhow!(i18n::t("errors.no_pkexec")))?;
            let mut command = Command::new(pkexec);
            command.arg(program);
            command
        }
        #[cfg(not(target_os = "linux"))]
        {
            return Err(anyhow!(i18n::t("errors.no_pkexec")));
        }
    } else {
        Command::new(program)
    };
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    Ok(command)
}

pub async fn run(args: &[String], elevated: bool, timeout: Duration) -> Result<Run> {
    let child = command(args, elevated)?.spawn()?;
    let output = tokio::time::timeout(timeout, child.wait_with_output())
        .await
        .map_err(|_| anyhow!(i18n::t("errors.timeout")))??;
    Ok(Run {
        ok: output.status.success(),
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// A long-running command whose output the caller reads as it comes.
pub fn spawn(args: &[String], elevated: bool) -> Result<Child> {
    Ok(command(args, elevated)?.spawn()?)
}

/// pkexec's own exit codes: the prompt was dismissed (126) or the password
/// refused (127). Either way the command never ran.
pub fn is_elevation_refused(elevated: bool, code: Option<i32>) -> bool {
    elevated && matches!(code, Some(126) | Some(127))
}

pub fn is_access_denied(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    text.contains("access denied") || text.contains("permission denied")
}

/// The part of a CLI failure worth showing: the first paragraph, without the
/// `sudo` advice the CLI appends — this app offers its own way through.
pub fn failure_text(text: &str) -> String {
    let first = text
        .split("\n\n")
        .map(str::trim)
        .find(|paragraph| !paragraph.is_empty())
        .unwrap_or("")
        .trim_start_matches("Error: ")
        .to_string();
    if first.chars().count() > 300 {
        let cut: String = first.chars().take(300).collect();
        format!("{cut}…")
    } else {
        first
    }
}

async fn cli_json(args: &[&str]) -> Result<Value> {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let run = run(&args, false, READ_TIMEOUT).await?;
    if !run.ok {
        return Err(anyhow!(failure_text(&run.text())));
    }
    Ok(serde_json::from_str(&run.stdout)?)
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

enum ReadError {
    Denied,
    Failed(anyhow::Error),
}

async fn read_status() -> Result<Value> {
    match localapi::get_json("/localapi/v0/status?peers=false").await {
        Ok(status) => Ok(status),
        Err(ApiError::Unreachable(_)) => cli_json(&["status", "--json", "--peers=false"]).await,
        Err(e) => Err(e.into()),
    }
}

async fn read_prefs() -> Result<Value> {
    match localapi::get_json("/localapi/v0/prefs").await {
        Ok(prefs) => Ok(prefs),
        Err(ApiError::Unreachable(_)) => cli_json(&["debug", "prefs"]).await,
        Err(e) => Err(e.into()),
    }
}

async fn read_accounts() -> Result<Vec<Account>, ReadError> {
    let list = match localapi::get_json("/localapi/v0/profiles/").await {
        Ok(list) => list,
        Err(ApiError::Denied(_)) => return Err(ReadError::Denied),
        Err(ApiError::Unreachable(_)) => {
            return match cli_json(&["switch", "--list", "--json"]).await {
                Ok(list) => Ok(accounts_from_cli(&list)),
                Err(e) if is_access_denied(&e.to_string()) => Err(ReadError::Denied),
                Err(e) => Err(ReadError::Failed(e)),
            };
        }
        Err(e) => return Err(ReadError::Failed(e.into())),
    };
    let current = localapi::get_json("/localapi/v0/profiles/current")
        .await
        .ok();
    let current_id = current.as_ref().and_then(|c| text_at(c, &["ID"]));
    Ok(accounts_from_api(&list, current_id.as_deref()))
}

/// Read everything, in one pass.
///
/// `previous` is the last list this app saw. When the daemon stops letting this
/// user list accounts — after a switch onto one with no operator — that list is
/// the best there is, and a window showing it with the way to fix access is
/// better than one that goes blank.
pub async fn snapshot(previous: &[Account]) -> Snapshot {
    if localapi::socket().is_none() && binary().is_none() {
        return Snapshot::unavailable(DaemonState::NotInstalled, None);
    }

    let status = match read_status().await {
        Ok(status) => status,
        Err(e) => {
            return Snapshot::unavailable(
                DaemonState::DaemonDown,
                Some(failure_text(&e.to_string())),
            )
        }
    };
    let mut connection = connection_from_status(&status);
    let prefs = read_prefs()
        .await
        .map(|p| prefs_from(&p))
        .unwrap_or_default();
    if let Some(id) = exit_node_id(&status) {
        connection.exit_node = exit_node_name(&id).await;
    }

    let (mut accounts, can_manage, complete) = match read_accounts().await {
        Ok(accounts) => (accounts, true, true),
        Err(ReadError::Denied) => (accounts_without_access(previous, &connection), false, false),
        Err(ReadError::Failed(e)) => {
            eprintln!("reading accounts: {e:#}");
            (accounts_without_access(previous, &connection), true, false)
        }
    };

    // The CLI does not report control servers; for the account in front, its
    // preferences do.
    if let (Some(active), Some(url)) = (accounts.iter_mut().find(|a| a.active), prefs.control_url) {
        active.control_url.get_or_insert(url);
    }

    Snapshot {
        connection,
        accounts,
        can_manage,
        problem: None,
        can_elevate: can_elevate(),
        login_needs_root: needs_root_for_new_accounts(),
        user: local_user(),
        complete,
    }
}

/// A non-empty string at `path`. The daemon writes empty strings for absent
/// values, and those are not worth carrying.
fn text_at(value: &Value, path: &[&str]) -> Option<String> {
    path.iter()
        .try_fold(value, |value, key| value.get(key))?
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn accounts_from_api(list: &Value, current: Option<&str>) -> Vec<Account> {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(|profile| {
            let id = text_at(profile, &["ID"])?;
            Some(Account {
                name: text_at(profile, &["Name"])
                    .or_else(|| text_at(profile, &["UserProfile", "LoginName"]))
                    .unwrap_or_else(|| id.clone()),
                login_name: text_at(profile, &["UserProfile", "LoginName"]),
                display_name: text_at(profile, &["UserProfile", "DisplayName"]),
                tailnet: text_at(profile, &["NetworkProfile", "DisplayName"])
                    .or_else(|| text_at(profile, &["NetworkProfile", "DomainName"])),
                control_url: text_at(profile, &["ControlURL"]),
                active: current == Some(id.as_str()),
                id,
            })
        })
        .collect()
}

fn accounts_from_cli(list: &Value) -> Vec<Account> {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(|profile| {
            let id = text_at(profile, &["id"])?;
            Some(Account {
                name: text_at(profile, &["nickname"])
                    .or_else(|| text_at(profile, &["account"]))
                    .unwrap_or_else(|| id.clone()),
                login_name: text_at(profile, &["account"]),
                display_name: None,
                tailnet: text_at(profile, &["tailnet"]),
                control_url: None,
                active: profile
                    .get("selected")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                id,
            })
        })
        .collect()
}

/// The last known list, with the active mark moved to whichever account the
/// status says is in front.
fn accounts_without_access(previous: &[Account], connection: &Connection) -> Vec<Account> {
    let mut accounts = previous.to_vec();
    for account in &mut accounts {
        account.active = connection.login_name.is_some()
            && account.login_name == connection.login_name
            && (connection.tailnet.is_none() || account.tailnet == connection.tailnet);
    }
    accounts
}

fn connection_from_status(status: &Value) -> Connection {
    let state = DaemonState::from_backend(status["BackendState"].as_str().unwrap_or(""));
    let ips: Vec<&str> = status["TailscaleIPs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let me = &status["Self"];
    let login_name = me["UserID"]
        .as_i64()
        .filter(|id| *id != 0)
        .and_then(|id| text_at(status, &["User", &id.to_string(), "LoginName"]));
    // A key that never expires comes without the field, or with Go's zero
    // time in it.
    let key_expiry = text_at(me, &["KeyExpiry"]).filter(|t| !t.starts_with("0001-"));

    Connection {
        state,
        version: text_at(status, &["Version"])
            .map(|v| v.split('-').next().unwrap_or(&v).to_string()),
        hostname: text_at(me, &["HostName"]),
        dns_name: text_at(me, &["DNSName"]).map(|n| n.trim_end_matches('.').to_string()),
        ipv4: ips
            .iter()
            .find(|ip| ip.contains('.'))
            .map(|ip| ip.to_string()),
        ipv6: ips
            .iter()
            .find(|ip| ip.contains(':'))
            .map(|ip| ip.to_string()),
        tailnet: text_at(status, &["CurrentTailnet", "Name"]),
        login_name,
        exit_node: None,
        key_expiry,
        auth_url: text_at(status, &["AuthURL"]),
    }
}

fn exit_node_id(status: &Value) -> Option<String> {
    text_at(status, &["ExitNodeStatus", "ID"])
}

fn prefs_from(prefs: &Value) -> Prefs {
    Prefs {
        control_url: text_at(prefs, &["ControlURL"]),
    }
}

/// The exit node's name. The status without peers only carries its id, and the
/// full status of a large tailnet is not something to fetch every few seconds,
/// so a name is looked up once per exit node and kept.
async fn exit_node_name(id: &str) -> Option<String> {
    static KNOWN: Mutex<Option<(String, String)>> = Mutex::new(None);

    if let Some((known_id, name)) = KNOWN.lock().ok()?.as_ref() {
        if known_id == id {
            return Some(name.clone());
        }
    }

    let status = match localapi::get_json("/localapi/v0/status").await {
        Ok(status) => status,
        Err(ApiError::Unreachable(_)) => cli_json(&["status", "--json"]).await.ok()?,
        Err(_) => return None,
    };
    let name = status["Peer"]
        .as_object()?
        .values()
        .find(|peer| text_at(peer, &["ID"]).as_deref() == Some(id))
        .and_then(|peer| {
            text_at(peer, &["HostName"]).or_else(|| {
                text_at(peer, &["DNSName"]).map(|n| n.split('.').next().unwrap_or("").to_string())
            })
        })?;

    if let Ok(mut known) = KNOWN.lock() {
        *known = Some((id.to_string(), name.clone()));
    }
    Some(name)
}

// ---------------------------------------------------------------------------
// Changing
// ---------------------------------------------------------------------------

/// Start a login on the account in front — for a key that expired, or an
/// account that was logged out.
///
/// Through the socket, not `tailscale up`: on an account that needs logging in,
/// `up` insists on being handed back every non-default setting the account has
/// (`--login-server` first of all, for anyone on Headscale) and refuses
/// otherwise. The socket's login keeps the settings as they are.
///
/// Returns whether the socket was there to do it; without one, the caller runs
/// `tailscale up` instead.
pub async fn start_interactive_login() -> Result<bool, ApiError> {
    if localapi::socket().is_none() {
        return Ok(false);
    }
    // Wanting to be connected is the point of logging in; otherwise an account
    // that had been turned off comes back authenticated and still off.
    localapi::call(
        "PATCH",
        "/localapi/v0/prefs",
        Some(&json!({ "WantRunning": true, "WantRunningSet": true })),
    )
    .await?;
    localapi::call("POST", "/localapi/v0/login-interactive", None).await?;
    Ok(true)
}

/// The login link, once the daemon has one.
pub async fn auth_url() -> Option<String> {
    let status = read_status().await.ok()?;
    text_at(&status, &["AuthURL"])
}

/// Tailscale's own control server, under the names it answers to.
pub fn is_default_server(url: &str) -> bool {
    matches!(
        url.trim_end_matches('/'),
        "https://controlplane.tailscale.com" | "https://login.tailscale.com"
    )
}

/// A control server as typed into the add dialog, made into a URL — or `None`
/// for Tailscale's own, which is what the CLI uses when given no flag.
pub fn normalize_server(input: Option<&str>) -> Result<Option<String>, String> {
    let Some(raw) = input.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let url = if raw.contains("://") {
        raw.to_string()
    } else {
        format!("https://{raw}")
    };
    let url = url.trim_end_matches('/').to_string();

    let host = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .map(|rest| rest.split('/').next().unwrap_or(""));
    let valid =
        matches!(host, Some(host) if !host.is_empty()) && !url.chars().any(char::is_whitespace);
    if !valid {
        return Err(i18n::t("errors.invalid_server"));
    }
    if is_default_server(&url) {
        return Ok(None);
    }
    Ok(Some(url))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api_profiles() -> Value {
        json!([
            {
                "ID": "4e04", "Name": "ernesto",
                "NetworkProfile": {"MagicDNSName": "ts.example.com", "DomainName": "tailnet.example.com", "DisplayName": ""},
                "UserProfile": {"ID": 1, "LoginName": "ernesto", "DisplayName": "Ernesto"},
                "ControlURL": "https://tailnet.example.com"
            },
            {
                "ID": "9a1f", "Name": "Work",
                "NetworkProfile": {"DomainName": "example.org", "DisplayName": "Example Org"},
                "UserProfile": {"LoginName": "me@example.org", "DisplayName": ""},
                "ControlURL": "https://controlplane.tailscale.com"
            }
        ])
    }

    #[test]
    fn reads_socket_profiles() {
        let accounts = accounts_from_api(&api_profiles(), Some("9a1f"));
        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[0].name, "ernesto");
        assert_eq!(accounts[0].tailnet.as_deref(), Some("tailnet.example.com"));
        assert_eq!(accounts[0].display_name.as_deref(), Some("Ernesto"));
        assert!(!accounts[0].active);
        assert_eq!(accounts[1].tailnet.as_deref(), Some("Example Org"));
        assert_eq!(accounts[1].display_name, None, "an empty string is no name");
        assert!(accounts[1].active);
    }

    #[test]
    fn reads_cli_profiles() {
        let list = json!([
            {"id": "4e04", "nickname": "ernesto", "tailnet": "tailnet.example.com", "account": "ernesto", "selected": true},
            {"id": "77aa", "nickname": "", "tailnet": "example.org", "account": "me@example.org", "selected": false}
        ]);
        let accounts = accounts_from_cli(&list);
        assert!(accounts[0].active);
        assert_eq!(
            accounts[1].name, "me@example.org",
            "no nickname falls back to the login"
        );
        assert_eq!(accounts[1].control_url, None);
    }

    #[test]
    fn reads_the_connection() {
        let status = json!({
            "Version": "1.102.3-t9329c3677-ga522f65e9",
            "BackendState": "Running",
            "AuthURL": "",
            "TailscaleIPs": ["100.64.0.2", "fd7a:115c:a1e0::2"],
            "Self": {"HostName": "laptop", "DNSName": "laptop.ts.example.com.", "UserID": 1, "KeyExpiry": "2026-10-01T10:00:00Z"},
            "User": {"1": {"LoginName": "ernesto"}},
            "CurrentTailnet": {"Name": "tailnet.example.com"},
            "ExitNodeStatus": {"ID": "n42"}
        });
        let c = connection_from_status(&status);
        assert_eq!(c.state, DaemonState::Running);
        assert_eq!(c.version.as_deref(), Some("1.102.3"));
        assert_eq!(c.ipv4.as_deref(), Some("100.64.0.2"));
        assert_eq!(c.ipv6.as_deref(), Some("fd7a:115c:a1e0::2"));
        assert_eq!(c.dns_name.as_deref(), Some("laptop.ts.example.com"));
        assert_eq!(c.login_name.as_deref(), Some("ernesto"));
        assert_eq!(c.auth_url, None);
        assert_eq!(exit_node_id(&status).as_deref(), Some("n42"));
    }

    #[test]
    fn a_logged_out_device_has_nothing_to_show() {
        let status = json!({"BackendState": "NeedsLogin", "AuthURL": "https://login.tailscale.com/a/abc", "Self": null});
        let c = connection_from_status(&status);
        assert_eq!(c.state, DaemonState::NeedsLogin);
        assert_eq!(
            c.auth_url.as_deref(),
            Some("https://login.tailscale.com/a/abc")
        );
        assert_eq!(c.hostname, None);
        assert_eq!(c.login_name, None);
    }

    #[test]
    fn without_access_the_last_list_stands() {
        let previous = accounts_from_api(&api_profiles(), Some("4e04"));
        let mut connection = Connection::empty(DaemonState::Running);
        connection.login_name = Some("me@example.org".into());
        connection.tailnet = Some("Example Org".into());
        let accounts = accounts_without_access(&previous, &connection);
        assert!(!accounts[0].active);
        assert!(accounts[1].active);
    }

    #[test]
    fn servers_are_normalized() {
        assert_eq!(normalize_server(None), Ok(None));
        assert_eq!(normalize_server(Some("  ")), Ok(None));
        assert_eq!(
            normalize_server(Some("headscale.example.com/")),
            Ok(Some("https://headscale.example.com".into()))
        );
        assert_eq!(
            normalize_server(Some("http://10.0.0.2:8080")),
            Ok(Some("http://10.0.0.2:8080".into()))
        );
        assert_eq!(
            normalize_server(Some("https://login.tailscale.com")),
            Ok(None)
        );
        assert!(normalize_server(Some("https://")).is_err());
        assert!(normalize_server(Some("ftp://x")).is_err());
        assert!(normalize_server(Some("https://a b")).is_err());
    }

    /// Reads the daemon on this machine — nothing is changed. Ignored by
    /// default, since it needs Tailscale installed and running:
    /// `cargo test live_snapshot -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_snapshot() {
        let snapshot = tauri::async_runtime::block_on(snapshot(&[]));
        println!("{}", serde_json::to_string_pretty(&snapshot).unwrap());
        assert!(snapshot.problem.is_none(), "{:?}", snapshot.problem);
    }

    #[test]
    fn failures_lose_the_sudo_advice() {
        let text = "Access denied: profiles access denied\n\nUse 'sudo tailscale switch --list'.\nTo not require root, use 'sudo tailscale set --operator=$USER' once.";
        assert_eq!(failure_text(text), "Access denied: profiles access denied");
        assert!(is_access_denied(text));
        assert!(!is_access_denied("Tailscale is stopped."));
    }
}
