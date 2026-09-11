//! The app's own settings, under `~/.config/tailscale-switch/state.json`.
//!
//! No credentials live here: the accounts are the daemon's. What is kept is
//! what the daemon has no notion of — the order the user dragged the list
//! into, a few window preferences, and control servers worth offering again.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::tailscale::{self, Account};

/// How many custom control servers the add dialog remembers.
const MAX_SERVERS: usize = 8;

#[derive(Serialize, Deserialize, Default, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
struct State {
    /// Account ids in the order the user dragged them into. Ids the daemon no
    /// longer knows are harmless and dropped at the next reorder.
    #[serde(default)]
    order: Vec<String>,
    /// Start with the window hidden, leaving only the tray icon.
    #[serde(default)]
    start_hidden: bool,
    /// UI language override as a tag (`it`, `en`). Absent means "follow the
    /// system", which is what a fresh install does.
    #[serde(default)]
    language: Option<String>,
    /// Control servers other than Tailscale's own, most recent first.
    #[serde(default)]
    servers: Vec<String>,
    /// The control server last seen for each account. The CLI only reports it
    /// for the account in front, so over the CLI this is how the others keep
    /// theirs.
    #[serde(default)]
    control_urls: HashMap<String, String>,
}

/// Every read-modify-write goes through here: the poll, the commands and the
/// tray all touch this file, from different threads.
static LOCK: Mutex<()> = Mutex::new(());

fn root() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .ok_or_else(|| anyhow!("cannot resolve the config directory"))?
        .join("tailscale-switch");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn path() -> Result<PathBuf> {
    Ok(root()?.join("state.json"))
}

fn read() -> Result<State> {
    let path = path()?;
    if !path.exists() {
        return Ok(State::default());
    }
    // A file that no longer parses is a settings reset, not a reason to stop.
    Ok(fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default())
}

/// Written beside and renamed over, so a crash mid-write leaves the old file
/// rather than half of a new one.
fn write(state: &State) -> Result<()> {
    let path = path()?;
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, serde_json::to_vec_pretty(state)?)?;
    fs::rename(&temp, &path)?;
    Ok(())
}

fn update(change: impl FnOnce(&mut State)) -> Result<()> {
    let _guard = LOCK.lock().map_err(|_| anyhow!("settings lock poisoned"))?;
    let before = read()?;
    let mut after = before.clone();
    change(&mut after);
    if after != before {
        write(&after)?;
    }
    Ok(())
}

fn get<T>(pick: impl FnOnce(State) -> T) -> Result<T> {
    let _guard = LOCK.lock().map_err(|_| anyhow!("settings lock poisoned"))?;
    Ok(pick(read()?))
}

pub fn start_hidden() -> Result<bool> {
    get(|s| s.start_hidden)
}

pub fn set_start_hidden(enabled: bool) -> Result<()> {
    update(|s| s.start_hidden = enabled)
}

pub fn language() -> Result<Option<String>> {
    get(|s| s.language)
}

pub fn set_language(tag: Option<&str>) -> Result<()> {
    update(|s| s.language = tag.map(str::to_string))
}

pub fn set_order(ids: &[String]) -> Result<()> {
    update(|s| s.order = ids.to_vec())
}

/// Put a server at the top of the list the add dialog offers.
pub fn remember_server(url: &str) -> Result<()> {
    if tailscale::is_default_server(url) {
        return Ok(());
    }
    update(|s| {
        s.servers.retain(|known| known != url);
        s.servers.insert(0, url.to_string());
        s.servers.truncate(MAX_SERVERS);
    })
}

/// The custom servers worth offering: the ones typed before, then any an
/// account on this machine uses.
pub fn servers(accounts: &[Account]) -> Result<Vec<String>> {
    let mut servers = get(|s| s.servers)?;
    for url in accounts.iter().filter_map(|a| a.control_url.as_deref()) {
        if !tailscale::is_default_server(url) && !servers.iter().any(|known| known == url) {
            servers.push(url.to_string());
        }
    }
    Ok(servers)
}

/// Put the accounts in the user's order and fill in the control servers the
/// read could not see — and remember the ones it could.
///
/// Accounts that were never dragged keep the daemon's order (oldest first)
/// after the ones that were, so a new login lands at the bottom.
///
/// `complete` says the list is the daemon's own, just read. Only then does an
/// account missing from it count as removed; a list kept from before, while the
/// daemon refuses to give one, proves nothing about what is gone.
pub fn arrange(accounts: &mut [Account], complete: bool) {
    let Ok(_guard) = LOCK.lock() else { return };
    let Ok(mut state) = read() else { return };
    let before = state.clone();

    accounts.sort_by_key(|account| {
        state
            .order
            .iter()
            .position(|id| id == &account.id)
            .unwrap_or(usize::MAX)
    });

    for account in accounts.iter_mut() {
        match &account.control_url {
            Some(url) => {
                state.control_urls.insert(account.id.clone(), url.clone());
            }
            None => account.control_url = state.control_urls.get(&account.id).cloned(),
        }
    }
    // An account removed from the daemon takes its server with it.
    if complete {
        state
            .control_urls
            .retain(|id, _| accounts.iter().any(|account| &account.id == id));
    }

    if state != before {
        if let Err(e) = write(&state) {
            eprintln!("saving settings: {e:#}");
        }
    }
}
