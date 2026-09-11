mod actions;
mod i18n;
mod localapi;
mod login;
mod notice;
mod store;
mod tailscale;
mod tray;

use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_autostart::ManagerExt;

use actions::Failure;
use tailscale::{DaemonState as Daemon, Snapshot};

/// How often the daemon is read over its socket: a local request costing
/// microseconds, so the tray can afford to notice a `tailscale switch` typed in
/// a terminal within a few seconds.
const POLL_SOCKET: Duration = Duration::from_secs(3);

/// Over the CLI every read is a handful of processes; half the pace is plenty.
const POLL_CLI: Duration = Duration::from_secs(6);

/// While something is settling — an account coming up, a login waiting for its
/// link to be used — so the window follows it closely.
const POLL_BUSY: Duration = Duration::from_secs(1);

/// The last snapshot read. The tray menu is rebuilt synchronously and cannot
/// wait on the daemon, so it draws from this.
struct Latest(Mutex<Snapshot>);

pub fn latest(app: &AppHandle) -> Snapshot {
    app.state::<Latest>()
        .0
        .lock()
        .map(|snapshot| snapshot.clone())
        .unwrap_or_else(|_| Snapshot::unavailable(Daemon::NoState, None))
}

/// Read the daemon, and tell the window and the tray if anything moved.
///
/// Called by the poll and after every change, so what the user just did shows
/// up without waiting for the next tick.
pub async fn refresh(app: &AppHandle) -> Snapshot {
    let previous = latest(app);
    let mut next = tailscale::snapshot(&previous.accounts).await;
    if !matches!(
        next.connection.state,
        Daemon::NotInstalled | Daemon::DaemonDown
    ) {
        store::arrange(&mut next.accounts, next.complete);
    }

    // Compared against what is stored now, not what was read before the await:
    // the poll and a command can refresh at the same time, and a change is
    // announced by whichever of them lands it first — once.
    let replaced = match app.state::<Latest>().0.lock() {
        Ok(mut slot) if *slot != next => Some(std::mem::replace(&mut *slot, next.clone())),
        _ => None,
    };
    if let Some(before) = replaced {
        announce(app, &before, &next);
        publish(app, &next);
    }
    next
}

/// Commands and the poll run off the main thread, and rebuilding the tray menu
/// touches GTK, which on Linux must happen on the main thread.
fn publish(app: &AppHandle, snapshot: &Snapshot) {
    refresh_tray(app);
    let _ = app.emit("state-changed", snapshot);
}

fn refresh_tray(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || tray::refresh(&handle));
}

/// Say what happened while nobody was looking.
///
/// One thing qualifies: the account in front losing its login on its own — a
/// node key that expired, typically hours into a session. Everything else is an
/// answer to something someone just pressed, or is already on the tray icon.
fn announce(app: &AppHandle, previous: &Snapshot, next: &Snapshot) {
    let (Some(before), Some(after)) = (previous.active(), next.active()) else {
        return;
    };
    let was_logged_in = matches!(
        previous.connection.state,
        Daemon::Running | Daemon::Starting | Daemon::Stopped
    );
    if before.id == after.id
        && was_logged_in
        && next.connection.state == Daemon::NeedsLogin
        && !app.state::<login::Jobs>().running()
    {
        notice::info(
            app,
            i18n::t_args("notices.needs_login", &[("name", &after.name)]),
        );
    }
}

async fn watch(app: AppHandle) {
    loop {
        let snapshot = refresh(&app).await;
        let settling =
            app.state::<login::Jobs>().running() || snapshot.connection.state == Daemon::Starting;
        let pace = if settling {
            POLL_BUSY
        } else if localapi::socket().is_some() {
            POLL_SOCKET
        } else {
            POLL_CLI
        };
        tokio::time::sleep(pace).await;
    }
}

/// What a tray item asks for. The tray has no room to report back, so
/// failures go out as notices — and a refusal the window can fix brings the
/// window up to fix it.
pub enum TrayAction {
    Switch(String),
    Connect,
    Disconnect,
    LogIn,
}

/// A change the window should retry once access has been granted.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase", tag = "kind")]
enum Retry {
    Switch { id: String },
    Connect,
    Disconnect,
    LogIn,
}

pub fn spawn_action(app: &AppHandle, action: TrayAction) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let (result, retry) = match action {
            TrayAction::Switch(id) => (actions::switch_to(&id).await, Retry::Switch { id }),
            TrayAction::Connect => (actions::connect().await, Retry::Connect),
            TrayAction::Disconnect => (actions::disconnect().await, Retry::Disconnect),
            TrayAction::LogIn => (actions::reauthenticate().await.map(|_| ()), Retry::LogIn),
        };
        refresh(&app).await;

        match result {
            Ok(()) => {}
            Err(failure) if failure.cancelled => {}
            Err(failure) if failure.access_denied => {
                tray::show_window(&app);
                let _ = app.emit("access-needed", retry);
            }
            Err(failure) => notice::error(
                &app,
                i18n::t_args("notices.action_failed", &[("error", &failure.message)]),
            ),
        }
    });
}

/// "Add account…" from the tray, for a window that was not listening yet.
#[derive(Default)]
struct AddRequested(AtomicBool);

pub fn request_add_dialog(app: &AppHandle) {
    app.state::<AddRequested>().0.store(true, Ordering::Relaxed);
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

async fn settled<T>(app: &AppHandle, result: Result<T, Failure>) -> Result<T, Failure> {
    refresh(app).await;
    result
}

#[tauri::command]
async fn get_state(app: AppHandle) -> Snapshot {
    refresh(&app).await
}

#[tauri::command]
async fn switch_account(app: AppHandle, id: String) -> Result<(), Failure> {
    let result = actions::switch_to(&id).await;
    settled(&app, result).await
}

#[tauri::command]
async fn remove_account(app: AppHandle, id: String) -> Result<(), Failure> {
    let active = latest(&app).active().map(|a| a.id.clone());
    let result = actions::remove(&id, active.as_deref()).await;
    settled(&app, result).await
}

#[tauri::command]
async fn logout(app: AppHandle) -> Result<(), Failure> {
    let result = actions::logout().await;
    settled(&app, result).await
}

#[tauri::command]
async fn connect(app: AppHandle) -> Result<(), Failure> {
    let result = actions::connect().await;
    settled(&app, result).await
}

#[tauri::command]
async fn disconnect(app: AppHandle) -> Result<(), Failure> {
    let result = actions::disconnect().await;
    settled(&app, result).await
}

#[tauri::command]
async fn rename_account(app: AppHandle, name: String) -> Result<(), Failure> {
    let result = actions::rename(&name).await;
    settled(&app, result).await
}

/// Start a login on the account in front; the link opens in the browser and
/// comes back for the window to offer again.
#[tauri::command]
async fn reauthenticate(app: AppHandle) -> Result<Option<String>, Failure> {
    let result = actions::reauthenticate().await;
    settled(&app, result).await
}

#[tauri::command]
async fn grant_access(app: AppHandle) -> Result<(), Failure> {
    let result = actions::grant_access().await;
    settled(&app, result).await
}

#[tauri::command]
async fn add_account(
    app: AppHandle,
    server: Option<String>,
    nickname: Option<String>,
    auth_key: Option<String>,
) -> Result<(), Failure> {
    login::start(&app, server, nickname, auth_key).await
}

#[tauri::command]
fn cancel_add_account(jobs: State<'_, login::Jobs>) {
    jobs.cancel();
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LoginStatus {
    running: bool,
    with_key: bool,
    progress: Option<login::Progress>,
}

/// Where a running login is, for a window that opens in the middle of one.
#[tauri::command]
fn login_status(jobs: State<'_, login::Jobs>) -> LoginStatus {
    LoginStatus {
        running: jobs.running(),
        with_key: jobs.with_key(),
        progress: jobs.progress(),
    }
}

#[tauri::command]
fn take_add_request(requested: State<'_, AddRequested>) -> bool {
    requested.0.swap(false, Ordering::Relaxed)
}

#[tauri::command]
fn open_url(url: String) -> Result<(), Failure> {
    actions::open_url(&url)
}

/// Control servers worth offering in the add dialog.
#[tauri::command]
fn known_servers(app: AppHandle) -> Vec<String> {
    store::servers(&latest(&app).accounts).unwrap_or_default()
}

#[tauri::command]
async fn reorder_accounts(app: AppHandle, ids: Vec<String>) -> Result<(), Failure> {
    let result = store::set_order(&ids).map_err(Failure::from);
    settled(&app, result).await
}

/// Everything the settings dialog shows, read in one round-trip.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Settings {
    start_hidden: bool,
    autostart: bool,
    /// The saved override, or `null` for "follow the system".
    language: Option<String>,
    /// What "follow the system" currently resolves to, so the dialog can name
    /// it instead of leaving the automatic option unexplained.
    system_language: String,
    /// The language actually in use, override or not.
    resolved_language: String,
    version: String,
}

#[tauri::command]
fn get_settings(app: AppHandle) -> Result<Settings, Failure> {
    Ok(Settings {
        start_hidden: store::start_hidden()?,
        // Absence of the autostart entry is a valid answer, not an error.
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        language: store::language()?,
        system_language: i18n::Lang::from_tag(&i18n::system_tag()).tag().to_string(),
        resolved_language: i18n::lang().tag().to_string(),
        version: app.package_info().version.to_string(),
    })
}

#[tauri::command]
fn set_start_hidden(enabled: bool) -> Result<(), Failure> {
    Ok(store::set_start_hidden(enabled)?)
}

#[tauri::command]
fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), Failure> {
    let manager = app.autolaunch();
    let result = if enabled {
        manager.enable()
    } else {
        manager.disable()
    };
    result.map_err(|e| {
        Failure::message(i18n::t_args(
            "errors.autostart",
            &[("error", &e.to_string())],
        ))
    })
}

/// `None` hands the choice back to the system locale.
#[tauri::command]
fn set_language(app: AppHandle, tag: Option<String>) -> Result<(), Failure> {
    let result = store::set_language(tag.as_deref());
    i18n::invalidate();
    // The tray menu is built from the same catalog, so it has to be redrawn.
    refresh_tray(&app);
    Ok(result?)
}

/// Whatever was raised while there was nowhere to show it.
#[tauri::command]
fn pending_notices(pending: State<'_, notice::Pending>) -> Vec<notice::Notice> {
    pending.take()
}

// ---------------------------------------------------------------------------
// Window and platform
// ---------------------------------------------------------------------------

/// Refuse the window manager's fullscreen, on the only layer that can see it.
///
/// `maximizable: false` in the config is a no-op under GTK, and
/// `Window::is_fullscreen` only reports what the app itself asked for, never
/// what a keyboard shortcut or a tiling extension did.
#[cfg(target_os = "linux")]
fn block_fullscreen(window: &tauri::WebviewWindow) {
    use gtk::gdk::WindowState;
    use gtk::glib;
    use gtk::prelude::*;

    let Ok(gtk_window) = window.gtk_window() else {
        eprintln!("cannot reach the GTK window: fullscreen stays possible");
        return;
    };
    gtk_window.connect_window_state_event(|window, event| {
        if event.new_window_state().contains(WindowState::FULLSCREEN) {
            // Undoing it from inside the signal handler races GTK's own
            // bookkeeping; one turn of the main loop later it sticks.
            let window = window.clone();
            glib::idle_add_local_once(move || window.unfullscreen());
        }
        glib::Propagation::Proceed
    });
}

/// Escape hatch for the rendering workarounds below.
#[cfg(target_os = "linux")]
const KEEP_WAYLAND: &str = "TAILSCALE_SWITCH_KEEP_WAYLAND";

/// WebKitGTK's DMA-BUF renderer and the NVIDIA proprietary driver disagree
/// under Wayland: the window tears and keeps stale tiles while it is dragged or
/// resized. Applied only where the NVIDIA module is loaded, and never over a
/// value the user set for themselves.
#[cfg(target_os = "linux")]
fn workaround_nvidia_rendering() {
    const VAR: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";
    if std::env::var_os(VAR).is_none() && std::path::Path::new("/sys/module/nvidia").exists() {
        std::env::set_var(VAR, "1");
    }
}

/// Run through XWayland on a Wayland session that uses fractional scaling.
///
/// GTK 3 has no fractional-scaling support, so under GNOME's
/// `scale-monitor-framebuffer` the compositor and the toolkit disagree about
/// how big the window is: clicks land beside the buttons, and a window dragged
/// onto a monitor with another scale comes back half drawn.
#[cfg(target_os = "linux")]
fn workaround_fractional_scaling() {
    if std::env::var_os(KEEP_WAYLAND).is_some() {
        return;
    }
    if std::env::var("XDG_SESSION_TYPE").as_deref() != Ok("wayland") {
        return;
    }
    let fractional = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.mutter", "experimental-features"])
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).contains("scale-monitor-framebuffer"))
        .unwrap_or(false);
    if fractional {
        std::env::set_var("GDK_BACKEND", "x11");
    }
}

/// Put the window away without letting the app go with it.
///
/// Queued on the main loop rather than run inside the close handler: on Linux
/// a hide issued from there can be swallowed, leaving the window on screen with
/// its close button apparently dead.
fn hide_to_tray(window: &tauri::Window) {
    let handle = window.app_handle().clone();
    let window = window.clone();
    let _ = handle.run_on_main_thread(move || {
        let _ = window.hide();
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "linux")]
    {
        workaround_fractional_scaling();
        workaround_nvidia_rendering();
    }

    tauri::Builder::default()
        // First, before anything else runs: a second launch has no business
        // building a tray icon. It brings the running app's window back and
        // exits.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            let handle = app.clone();
            let _ = app.run_on_main_thread(move || tray::show_window(&handle));
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        // Rust-side only: nothing in the webview posts a notification.
        .plugin(tauri_plugin_notification::init())
        .manage(Latest(Mutex::new(Snapshot::unavailable(
            Daemon::NoState,
            None,
        ))))
        .manage(login::Jobs::default())
        .manage(notice::Pending::default())
        .manage(AddRequested::default())
        .invoke_handler(tauri::generate_handler![
            get_state,
            switch_account,
            remove_account,
            logout,
            connect,
            disconnect,
            rename_account,
            reauthenticate,
            grant_access,
            add_account,
            cancel_add_account,
            login_status,
            take_add_request,
            open_url,
            known_servers,
            reorder_accounts,
            get_settings,
            set_start_hidden,
            set_autostart,
            set_language,
            pending_notices,
        ])
        .setup(|app| {
            tray::build(app.handle())?;

            #[cfg(target_os = "linux")]
            if let Some(window) = app.get_webview_window("main") {
                block_fullscreen(&window);
            }

            // The window is created hidden so that starting in the tray never
            // flashes it; show it now unless that is what the user asked for.
            if !store::start_hidden().unwrap_or(false) {
                tray::show_window(app.handle());
            }

            let handle = app.handle().clone();
            tauri::async_runtime::spawn(watch(handle));
            Ok(())
        })
        .on_window_event(|window, event| match event {
            // Closing the window leaves the app alive in the tray.
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                hide_to_tray(window);
            }
            #[cfg(not(target_os = "linux"))]
            WindowEvent::Resized(_) => {
                if window.is_fullscreen().unwrap_or(false) {
                    let _ = window.set_fullscreen(false);
                }
            }
            _ => {}
        })
        .build(tauri::generate_context!())
        .expect("error while building the application")
        .run(|app, event| match event {
            tauri::RunEvent::ExitRequested { api, code, .. } => {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => tray::show_window(app),
            _ => {
                let _ = app;
            }
        });
}
