//! System tray icon and its account menu.

use std::sync::atomic::{AtomicU8, Ordering};
use tauri::{
    image::Image,
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, Wry,
};

use crate::i18n;
use crate::tailscale::{DaemonState, Snapshot};

pub const TRAY_ID: &str = "main";
const ACCOUNT_PREFIX: &str = "account:";

/// The switch glyph rather than the app icon: in a panel full of app icons,
/// what this one does reads better than which app it is. On Linux it carries
/// its own plate, since a panel can be light or dark.
#[cfg(not(target_os = "macos"))]
const ICON_ON: &[u8] = include_bytes!("../icons/tray.png");
#[cfg(not(target_os = "macos"))]
const ICON_OFF: &[u8] = include_bytes!("../icons/tray-off.png");
/// The menu bar tints a template itself, so there the glyph goes bare.
#[cfg(target_os = "macos")]
const ICON_ON: &[u8] = include_bytes!("../icons/tray-template.png");
#[cfg(target_os = "macos")]
const ICON_OFF: &[u8] = include_bytes!("../icons/tray-off-template.png");

/// Which icon the tray shows: 0 unset, 1 connected, 2 anything else. Swapped
/// only when it changes — GTK complains about redundant icon writes, and the
/// menu is rebuilt often.
static SHOWING: AtomicU8 = AtomicU8::new(0);

pub fn build(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let builder = TrayIconBuilder::with_id(TRAY_ID)
        .icon(Image::from_bytes(ICON_OFF)?)
        .tooltip(i18n::t("app.name"))
        .menu(&menu(app, &crate::latest(app))?)
        .on_menu_event(on_menu_event);

    #[cfg(target_os = "macos")]
    let builder = builder.icon_as_template(true);

    builder.build(app)?;
    SHOWING.store(2, Ordering::Relaxed);
    Ok(())
}

fn status_line(snapshot: &Snapshot) -> String {
    let active = snapshot.active();
    match snapshot.connection.state {
        DaemonState::NotInstalled => i18n::t("tray.status_not_installed"),
        DaemonState::DaemonDown => i18n::t("tray.status_daemon_down"),
        _ if active.is_none() => i18n::t("tray.status_no_account"),
        DaemonState::Running => i18n::t_args(
            "tray.status_running",
            &[("name", active.map(|a| a.name.as_str()).unwrap_or(""))],
        ),
        DaemonState::Starting | DaemonState::NoState => i18n::t("tray.status_starting"),
        DaemonState::Stopped => i18n::t("tray.status_stopped"),
        DaemonState::NeedsLogin => i18n::t("tray.status_needs_login"),
        DaemonState::NeedsMachineAuth => i18n::t("tray.status_needs_machine_auth"),
    }
}

/// `name  ·  tailnet`, with the tailnet left out when it only repeats the name.
fn account_label(name: &str, tailnet: Option<&str>) -> String {
    match tailnet {
        Some(tailnet) if tailnet != name => format!("{name}  ·  {tailnet}"),
        _ => name.to_string(),
    }
}

fn menu(app: &AppHandle, snapshot: &Snapshot) -> tauri::Result<Menu<Wry>> {
    let menu = Menu::new(app)?;
    let reachable = !matches!(
        snapshot.connection.state,
        DaemonState::NotInstalled | DaemonState::DaemonDown
    );

    menu.append(&MenuItem::with_id(
        app,
        "status",
        status_line(snapshot),
        false,
        None::<&str>,
    )?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;

    if reachable {
        if snapshot.accounts.is_empty() {
            menu.append(&MenuItem::with_id(
                app,
                "noop",
                i18n::t("tray.no_accounts"),
                false,
                None::<&str>,
            )?)?;
        }
        for account in &snapshot.accounts {
            menu.append(&CheckMenuItem::with_id(
                app,
                format!("{ACCOUNT_PREFIX}{}", account.id),
                account_label(&account.name, account.tailnet.as_deref()),
                true,
                account.active,
                None::<&str>,
            )?)?;
        }
        menu.append(&PredefinedMenuItem::separator(app)?)?;

        if snapshot.active().is_some() {
            let toggle = match snapshot.connection.state {
                DaemonState::Running | DaemonState::Starting => {
                    Some(("disconnect", "tray.disconnect"))
                }
                DaemonState::Stopped => Some(("connect", "tray.connect")),
                DaemonState::NeedsLogin => Some(("log-in", "tray.log_in")),
                _ => None,
            };
            if let Some((id, key)) = toggle {
                menu.append(&MenuItem::with_id(
                    app,
                    id,
                    i18n::t(key),
                    true,
                    None::<&str>,
                )?)?;
            }
        }
        menu.append(&MenuItem::with_id(
            app,
            "add",
            i18n::t("tray.add_account"),
            true,
            None::<&str>,
        )?)?;
    }

    menu.append(&MenuItem::with_id(
        app,
        "open",
        i18n::t("tray.open_window"),
        true,
        None::<&str>,
    )?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(
        app,
        "quit",
        i18n::t("tray.quit"),
        true,
        None::<&str>,
    )?)?;

    Ok(menu)
}

/// Rebuild the menu, the icon and the tooltip from the latest snapshot. Must
/// run on the main thread: it touches GTK.
pub fn refresh(app: &AppHandle) {
    let snapshot = crate::latest(app);
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    if let Ok(menu) = menu(app, &snapshot) {
        let _ = tray.set_menu(Some(menu));
    }

    let connected =
        snapshot.connection.state == DaemonState::Running && snapshot.active().is_some();
    let wanted = if connected { 1 } else { 2 };
    if SHOWING.swap(wanted, Ordering::Relaxed) != wanted {
        if let Ok(image) = Image::from_bytes(if connected { ICON_ON } else { ICON_OFF }) {
            let _ = tray.set_icon(Some(image));
            #[cfg(target_os = "macos")]
            let _ = tray.set_icon_as_template(true);
        }
    }

    let tooltip = match snapshot.active() {
        Some(active) => format!(
            "{}\n{}",
            i18n::t("app.name"),
            account_label(&active.name, active.tailnet.as_deref())
        ),
        None => i18n::t("app.name"),
    };
    let _ = tray.set_tooltip(Some(format!("{tooltip}\n{}", status_line(&snapshot))));
}

fn on_menu_event(app: &AppHandle, event: tauri::menu::MenuEvent) {
    let id = event.id().as_ref().to_string();
    match id.as_str() {
        "quit" => app.exit(0),
        "open" => show_window(app),
        "add" => {
            show_window(app);
            // The window opens its add dialog on this; a window that was not
            // up yet asks for it on the way in instead.
            crate::request_add_dialog(app);
            let _ = app.emit("open-add-account", ());
        }
        "connect" => crate::spawn_action(app, crate::TrayAction::Connect),
        "disconnect" => crate::spawn_action(app, crate::TrayAction::Disconnect),
        "log-in" => crate::spawn_action(app, crate::TrayAction::LogIn),
        _ => {
            if let Some(account) = id.strip_prefix(ACCOUNT_PREFIX) {
                crate::spawn_action(app, crate::TrayAction::Switch(account.to_string()));
            }
        }
    }
}

pub fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tailnet_is_left_out_when_it_repeats_the_name() {
        assert_eq!(
            account_label("work", Some("example.org")),
            "work  ·  example.org"
        );
        assert_eq!(
            account_label("example.org", Some("example.org")),
            "example.org"
        );
        assert_eq!(account_label("work", None), "work");
    }
}
