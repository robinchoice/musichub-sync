// Tray app: the sync runs in a background thread, the popover reads its status and starts actions.
// The session token stays in the system keychain and on the Rust side; the popover never sees it.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use musichub_sync_core::hub::{Error, HttpHub};
use musichub_sync_core::login;
use musichub_sync_core::plan::SkipReason;
use musichub_sync_core::run::{Problem, Syncer};
use serde::{Deserialize, Serialize};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_positioner::{Position, WindowExt};

const DEFAULT_SERVER: &str = "https://hub.pleasance.org";
const ROUND: Duration = Duration::from_secs(20);
const KEYRING_SERVICE: &str = "org.pleasance.musichubsync";

#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Config {
    server: Option<String>,
    folder: Option<PathBuf>,
    email: Option<String>,
    excluded: Vec<String>,
    paused: bool,
    // Set once the user went through the setup
    ready: bool,
}

impl Config {
    fn server(&self) -> String {
        self.server.clone().unwrap_or_else(|| std::env::var("MUSICHUB_URL").unwrap_or_else(|_| DEFAULT_SERVER.into()))
    }
}

#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct ProjectInfo {
    id: String,
    name: String,
    tracks: usize,
    bytes: u64,
}

#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct Status {
    // login | setup | synced | syncing | paused | offline | error
    state: String,
    message: Option<String>,
    last_sync: Option<u64>,
    busy: usize,
    problems: Vec<ProblemInfo>,
    projects: Vec<ProjectInfo>,
}

#[derive(Clone, Serialize)]
struct ProblemInfo {
    path: String,
    message: String,
}

impl From<&Problem> for ProblemInfo {
    fn from(p: &Problem) -> Self {
        ProblemInfo { path: p.path.clone(), message: p.message.clone() }
    }
}

struct Shared {
    config: Mutex<Config>,
    status: Mutex<Status>,
    kick: Mutex<Sender<()>>,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn config_file(app: &AppHandle) -> PathBuf {
    app.path().app_config_dir().unwrap_or_default().join("config.json")
}

fn index_file(app: &AppHandle) -> PathBuf {
    app.path().app_data_dir().unwrap_or_default().join("index.json")
}

fn load_config(app: &AppHandle) -> Config {
    std::fs::read(config_file(app)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save_config(app: &AppHandle, config: &Config) {
    let file = config_file(app);
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(file, serde_json::to_vec_pretty(config).unwrap_or_default());
}

fn keyring() -> Option<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, "session").ok()
}

fn token() -> Option<String> {
    keyring()?.get_password().ok()
}

fn default_folder(app: &AppHandle) -> PathBuf {
    app.path().home_dir().unwrap_or_default().join("Music Hub")
}

fn set_status(app: &AppHandle, f: impl FnOnce(&mut Status)) {
    let shared = app.state::<Shared>();
    let status = {
        let mut status = shared.status.lock().unwrap();
        f(&mut status);
        status.clone()
    };
    update_tray(app, &status);
    let _ = app.emit("status", status);
}

fn update_tray(app: &AppHandle, status: &Status) {
    if let Some(tray) = app.tray_by_id("main") {
        let tooltip = match status.state.as_str() {
            "synced" => "Music Hub: alles synchron".to_string(),
            "syncing" => "Music Hub: gleicht ab …".to_string(),
            "paused" => "Music Hub: pausiert".to_string(),
            "login" | "setup" => "Music Hub: einrichten".to_string(),
            _ => format!("Music Hub: {}", status.message.clone().unwrap_or_default()),
        };
        let _ = tray.set_tooltip(Some(tooltip));
    }
}

// The sync thread: a round every ROUND, or right away when kicked
fn sync_loop(app: AppHandle, kicks: Receiver<()>) {
    let mut syncer: Option<(String, PathBuf, Syncer<HttpHub>)> = None;
    loop {
        let config = app.state::<Shared>().config.lock().unwrap().clone();
        match (token(), &config.folder) {
            (None, _) => set_status(&app, |s| s.state = "login".into()),
            (Some(_), _) if !config.ready => set_status(&app, |s| s.state = "setup".into()),
            (Some(_), None) => set_status(&app, |s| s.state = "setup".into()),
            (Some(_), _) if config.paused => set_status(&app, |s| s.state = "paused".into()),
            (Some(token), Some(folder)) => {
                let server = config.server();
                let fresh = !matches!(&syncer, Some((s, f, _)) if *s == server && f == folder);
                if fresh {
                    syncer = Some((server.clone(), folder.clone(), Syncer::new(HttpHub::new(&server, &token), folder, &index_file(&app))));
                }
                let (_, _, s) = syncer.as_mut().unwrap();
                set_status(&app, |st| st.state = "syncing".into());
                let result = s.run(&config.excluded, now());
                let projects = s
                    .state()
                    .map(|state| {
                        state
                            .projects
                            .iter()
                            .map(|p| ProjectInfo {
                                id: p.id.clone(),
                                name: p.name.clone(),
                                tracks: p.tracks.len(),
                                bytes: p.tracks.iter().map(|t| t.versions.iter().map(|v| v.file_size).sum::<u64>() + t.stems.iter().map(|s| s.file_size).sum::<u64>()).sum(),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                match result {
                    Ok(report) => {
                        notify(&app, &report.uploaded, &report.downloaded);
                        let mut problems: Vec<ProblemInfo> = report.problems.iter().map(Into::into).collect();
                        problems.extend(report.skipped.iter().filter(|(_, r)| *r == SkipReason::NoUploadRights).map(|(path, _)| ProblemInfo {
                            path: path.clone(),
                            message: "Keine Upload-Rechte in diesem Projekt, die Datei bleibt lokal".into(),
                        }));
                        set_status(&app, |st| {
                            st.state = if report.busy > 0 { "syncing".into() } else { "synced".into() };
                            st.message = None;
                            st.last_sync = Some(now());
                            st.busy = report.busy;
                            st.problems = problems;
                            st.projects = projects;
                        });
                    }
                    Err(Error::Unauthorized) => {
                        if let Some(entry) = keyring() {
                            let _ = entry.delete_credential();
                        }
                        syncer = None;
                        set_status(&app, |st| st.state = "login".into());
                    }
                    Err(Error::Network(message)) => set_status(&app, |st| {
                        st.state = "offline".into();
                        st.message = Some(message);
                    }),
                    Err(e) => set_status(&app, |st| {
                        st.state = "error".into();
                        st.message = Some(e.to_string());
                    }),
                }
            }
        }
        let _ = kicks.recv_timeout(ROUND);
        while kicks.try_recv().is_ok() {}
    }
}

fn notify(app: &AppHandle, uploaded: &[String], downloaded: &[String]) {
    let name = |path: &String| path.rsplit('/').next().unwrap_or(path).to_string();
    let show = |title: String, body: String| {
        let _ = app.notification().builder().title(title).body(body).show();
    };
    match uploaded {
        [] => {}
        [one] => show("Im Hub".into(), format!("{} ist hochgeladen", name(one))),
        many => show("Im Hub".into(), format!("{} Dateien hochgeladen", many.len())),
    }
    match downloaded {
        [] => {}
        [one] => show("Neu in deinem Ordner".into(), name(one)),
        many => show("Neu in deinem Ordner".into(), format!("{} Dateien", many.len())),
    }
}

fn kick(shared: &Shared) {
    let _ = shared.kick.lock().unwrap().send(());
}

#[tauri::command]
fn status(shared: State<Shared>, app: AppHandle) -> serde_json::Value {
    let config = shared.config.lock().unwrap().clone();
    let status = shared.status.lock().unwrap().clone();
    serde_json::json!({
        "status": status,
        "server": config.server(),
        "email": config.email,
        "folder": config.folder.unwrap_or_else(|| default_folder(&app)),
        "excluded": config.excluded,
        "paused": config.paused,
        "version": env!("CARGO_PKG_VERSION"),
    })
}

// Starts the device login and polls in the background; the popover hears "status" when it's done
#[tauri::command]
fn login_start(app: AppHandle, shared: State<Shared>) -> Result<serde_json::Value, String> {
    let server = shared.config.lock().unwrap().server();
    let code = login::start(&server).map_err(|e| e.to_string())?;
    let _ = app.opener().open_url(&code.verification_url, None::<&str>);
    let polling = code.clone();
    thread::spawn(move || {
        for _ in 0..(600 / polling.poll_interval_seconds.max(1)) {
            thread::sleep(Duration::from_secs(polling.poll_interval_seconds.max(1)));
            match login::poll(&server, &polling) {
                Ok(None) => continue,
                Ok(Some(session)) => {
                    if let Some(entry) = keyring() {
                        let _ = entry.set_password(&session.token);
                    }
                    let shared = app.state::<Shared>();
                    let config = {
                        let mut config = shared.config.lock().unwrap();
                        config.email = Some(session.user.email);
                        config.clone()
                    };
                    save_config(&app, &config);
                    kick(&shared);
                    return;
                }
                Err(e) => {
                    let _ = app.emit("login-failed", e.to_string());
                    return;
                }
            }
        }
        let _ = app.emit("login-failed", "Der Code ist abgelaufen".to_string());
    });
    // The device code stays here, the popover only shows what the user types
    Ok(serde_json::json!({ "userCode": code.user_code, "verificationUrl": code.verification_url }))
}

#[tauri::command]
fn open_verification(app: AppHandle, url: String) {
    let _ = app.opener().open_url(url, None::<&str>);
}

// Finishes the setup or changes folder and projects later
#[tauri::command]
fn configure(app: AppHandle, shared: State<Shared>, folder: Option<PathBuf>, excluded: Option<Vec<String>>, paused: Option<bool>) {
    let config = {
        let mut config = shared.config.lock().unwrap();
        if let Some(folder) = folder {
            config.folder = Some(folder);
        }
        if let Some(excluded) = excluded {
            config.excluded = excluded;
        }
        if let Some(paused) = paused {
            config.paused = paused;
        }
        config.ready = config.folder.is_some();
        config.clone()
    };
    save_config(&app, &config);
    kick(&shared);
}

// Projects to choose from during the setup, before the first round
#[tauri::command]
fn projects(shared: State<Shared>) -> Result<Vec<ProjectInfo>, String> {
    let server = shared.config.lock().unwrap().server();
    let token = token().ok_or("Nicht angemeldet")?;
    let hub = HttpHub::new(&server, &token);
    use musichub_sync_core::hub::Hub;
    let state = hub.state().map_err(|e| e.to_string())?.unwrap_or_default();
    Ok(state
        .projects
        .iter()
        .map(|p| ProjectInfo {
            id: p.id.clone(),
            name: p.name.clone(),
            tracks: p.tracks.len(),
            bytes: p.tracks.iter().map(|t| t.versions.iter().map(|v| v.file_size).sum::<u64>() + t.stems.iter().map(|s| s.file_size).sum::<u64>()).sum(),
        })
        .collect())
}

#[tauri::command]
fn activity(shared: State<Shared>) -> Result<serde_json::Value, String> {
    let server = shared.config.lock().unwrap().server();
    let token = token().ok_or("Nicht angemeldet")?;
    HttpHub::new(&server, &token).activity(8).map_err(|e| e.to_string())
}

#[tauri::command]
fn sync_now(shared: State<Shared>) {
    kick(&shared);
}

#[tauri::command]
fn open_folder(app: AppHandle, shared: State<Shared>, path: Option<String>) {
    let folder = shared.config.lock().unwrap().folder.clone().unwrap_or_else(|| default_folder(&app));
    let _ = std::fs::create_dir_all(&folder);
    let target = match path {
        Some(p) => folder.join(p),
        None => folder,
    };
    let _ = app.opener().open_path(target.to_string_lossy(), None::<&str>);
}

#[tauri::command]
fn open_hub(app: AppHandle, shared: State<Shared>, path: Option<String>) {
    let server = shared.config.lock().unwrap().server();
    let _ = app.opener().open_url(format!("{server}{}", path.unwrap_or_default()), None::<&str>);
}

#[tauri::command]
fn logout(app: AppHandle, shared: State<Shared>) {
    if let Some(entry) = keyring() {
        let _ = entry.delete_credential();
    }
    let config = {
        let mut config = shared.config.lock().unwrap();
        config.email = None;
        config.clone()
    };
    save_config(&app, &config);
    kick(&shared);
}

#[tauri::command]
fn quit(app: AppHandle) {
    app.exit(0);
}

fn toggle_popover(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
        } else {
            let _ = window.move_window(if cfg!(target_os = "windows") { Position::TrayBottomCenter } else { Position::TrayCenter });
            let _ = window.show();
            let _ = window.set_focus();
            let _ = app.emit("opened", ());
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let (kick_tx, kick_rx) = mpsc::channel();
    let kicks = Arc::new(Mutex::new(Some(kick_rx)));

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| toggle_popover(app)))
        .plugin(tauri_plugin_positioner::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .invoke_handler(tauri::generate_handler![
            status,
            login_start,
            open_verification,
            configure,
            projects,
            activity,
            sync_now,
            open_folder,
            open_hub,
            logout,
            quit
        ])
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let handle = app.handle().clone();
            let config = load_config(&handle);
            let first_run = !config.ready || token().is_none();
            app.manage(Shared { config: Mutex::new(config), status: Mutex::new(Status::default()), kick: Mutex::new(kick_tx) });

            // A template icon follows the menu bar's colours on macOS; elsewhere the light wave suits dark panels
            let icon = if cfg!(target_os = "macos") {
                Image::from_bytes(include_bytes!("../icons/tray.png"))?
            } else {
                Image::from_bytes(include_bytes!("../icons/tray-light.png"))?
            };
            let menu = Menu::with_items(app, &[&MenuItem::with_id(app, "open", "Öffnen", true, None::<&str>)?, &MenuItem::with_id(app, "quit", "Beenden", true, None::<&str>)?])?;
            TrayIconBuilder::with_id("main")
                .icon(icon)
                .icon_as_template(cfg!(target_os = "macos"))
                .tooltip("Music Hub")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "quit" => app.exit(0),
                    _ => toggle_popover(app),
                })
                .on_tray_icon_event(|tray, event| {
                    tauri_plugin_positioner::on_tray_event(tray.app_handle(), &event);
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                        toggle_popover(tray.app_handle());
                    }
                })
                .build(app)?;

            if let Some(window) = app.get_webview_window("main") {
                let popover = window.clone();
                window.on_window_event(move |event| {
                    // A popover closes when you click elsewhere, except during the setup
                    if let WindowEvent::Focused(false) = event {
                        let state = popover.app_handle().state::<Shared>().status.lock().unwrap().state.clone();
                        if state != "login" && state != "setup" {
                            let _ = popover.hide();
                        }
                    }
                });
                if first_run {
                    let _ = window.center();
                    let _ = window.show();
                }
            }

            let rx = kicks.lock().unwrap().take().expect("receiver");
            thread::spawn(move || sync_loop(handle, rx));
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Music Hub Sync");
}
