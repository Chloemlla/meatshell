//! Top-level UI state machine.
//!
//! Responsibilities:
//!   * Load the config store and expose sessions to Slint.
//!   * Drive the 1-Hz system sampler.
//!   * Manage the tab list + per-tab `SessionHandle` map.
//!   * Route Slint callbacks to the right domain module.
mod auth_dialogs;
mod aux_windows;
pub(crate) mod core;
mod dock_stacks;
mod file_drop;
mod fonts;
mod helpers;
mod hit_test;
#[path = "app/editor_syntax.rs"]
mod editor_syntax;
#[cfg(windows)]
mod jump_list;
mod key_input;
pub mod launch;
mod misc_callbacks;
mod open_window;
mod panes;
mod port_forward;
mod quick_commands;
mod resource_ui;
mod session_callbacks;
mod session_event;
mod session_editor;
mod session_models;
mod session_runtime;
mod session_trigger;
mod settings_callbacks;
mod sftp_callbacks;
mod sftp_ui;
mod sidebar;
mod single_instance;
mod tab_callbacks;
mod tab_transfer;
mod term_output;
mod terminal_ui;
mod tray;
mod tunnel_callbacks;
mod webdav;
mod window;
mod window_events;

use self::auth_dialogs::*;
use self::aux_windows::*;
use self::dock_stacks::*;
use self::file_drop::*;
use self::fonts::*;
use self::helpers::*;
use self::hit_test::*;
use self::key_input::*;
use self::misc_callbacks::*;
use self::open_window::*;
use self::panes::*;
use self::port_forward::*;
use self::quick_commands::*;
use self::resource_ui::*;
use self::session_callbacks::*;
use self::session_event::*;
use self::session_models::*;
use self::session_runtime::*;
use self::session_trigger::*;
use self::settings_callbacks::*;
use self::sftp_callbacks::*;
use self::sftp_ui::*;
use self::sidebar::*;
use self::tab_callbacks::*;
use self::tab_transfer::*;
use self::term_output::*;
use self::terminal_ui::*;
use self::tunnel_callbacks::*;
use self::webdav::*;
use self::window::*;
use self::window_events::*;

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::sync::{Arc, Mutex, OnceLock};

#[cfg(test)]
#[path = "../tests/app/terminal_ingest/mod.rs"]
mod ingest_frame_tests;

use anyhow::{Context, Result};
use i_slint_backend_winit::WinitWindowAccessor;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use tokio::runtime::Runtime;

use crate::app::core::{AppCore, TabRoute, TabRoutes, WindowRegistry, WindowState};
use crate::config::{
    is_reserved_session_group, named_display_groups, AuthMethod, ConfigStore, OutputHighlightRule,
    Secret, Session, SessionKind, SessionLogMode,
};
use crate::i18n::t;
use crate::layout::{LogicalRect, TerminalWheelHit};
use crate::resource::system::{format_bytes_per_sec, format_mem};
use crate::resource::{LocalSnap, NetHist, TabStatus, TabStatuses};
use crate::resource::{SystemSampler, SystemSnapshot};
use crate::session::{ConnectCtx, PendingCred, PendingHostKey, PendingMfa};
use crate::sftp::{download_target_path, spawn_sftp, DownloadConflict, SftpHandles, SftpLastCwd};
use crate::ssh::{
    format_mtime, format_size, spawn_session, test_session_auth, ProcInfo, SessionCommand,
    SessionEvent, SessionHandle, SystemDetails,
};
#[cfg(windows)]
use crate::terminal::c0_letter_key_down;
use crate::terminal::{
    bare_ctrl_marker_workaround_enabled, cell_prefix, clear_pending_paste, compile_output_rules,
    encode_command_bar_input, encode_mouse_event, encode_pasted_text, is_back_tab,
    is_terminal_interrupt, key_to_pty_bytes, paste_requires_large_review,
    should_drop_bare_ctrl_marker, store_pending_paste, take_pending_paste,
    terminal_uses_bracketed_paste, CsiState, OutputHighlightPreset, PendingPaste, RenderGates,
    TabRenderGate, TermBuffer, TermBufferHandle, TermBuffers, BACK_TAB_BYTES,
};
#[cfg(test)]
use crate::terminal::{
    build_row, highlight_plain_output, log_level_marker, normalize_pasted_newlines,
    text_cell_width, vt_span_colors, CompiledOutputRule, HistSpan, Line,
};
#[cfg(any(target_os = "windows", test))]
use crate::terminal::{windows_process_ctrl_release, CtrlKeySide};
use crate::ui::*;
use crate::webdav::WebDavAcceptAnyCertVerifier;

/// Persist the config immediately, logging (instead of silently dropping) any
/// write error. Silent write failures meant sessions/settings were lost on
/// disk-full / permission errors (#8).
fn persist_config(store: &ConfigStore) {
    if let Err(e) = store.save() {
        tracing::warn!("failed to save config: {e:#}");
    }
}

/// Debounced config writer (#3): routine UI-triggered mutations coalesce into
/// one write per quiet window instead of a full rewrite per event. Close-path
/// saves bypass this (via `persist_config`) so the final write is never lost.
struct ConfigWriter {
    store: Rc<RefCell<ConfigStore>>,
    dirty: Cell<bool>,
    armed: Cell<bool>,
}

thread_local! {
    static CONFIG_WRITER: RefCell<Option<Rc<ConfigWriter>>> = RefCell::new(None);
}

impl ConfigWriter {
    fn register(store: Rc<RefCell<ConfigStore>>) {
        CONFIG_WRITER.with(|w| {
            *w.borrow_mut() = Some(Rc::new(ConfigWriter {
                store,
                dirty: Cell::new(false),
                armed: Cell::new(false),
            }));
        });
    }

    fn schedule_save(self: &Rc<Self>) {
        self.dirty.set(true);
        if self.armed.replace(true) {
            return; // a write is already pending and will pick this change up
        }
        let me = self.clone();
        slint::Timer::single_shot(std::time::Duration::from_millis(250), move || {
            me.flush();
        });
    }

    fn flush(&self) {
        self.armed.set(false);
        if self.dirty.replace(false) {
            persist_config(&self.store.borrow());
        }
    }
}

/// Queue a debounced config save for the process store (#3).
fn schedule_config_save() {
    CONFIG_WRITER.with(|w| {
        if let Some(writer) = w.borrow().as_ref() {
            writer.schedule_save();
        }
    });
}

/// Single background clipboard writer (#4). Every copy is routed through one
/// worker thread so arboard's `set().wait()` (which on Linux blocks until
/// another app takes clipboard ownership) can never accumulate threads.
struct ClipboardWorker {
    pending: Mutex<Option<String>>,
    cv: std::sync::Condvar,
}

impl ClipboardWorker {
    fn spawn() -> Arc<Self> {
        let worker = Arc::new(ClipboardWorker {
            pending: Mutex::new(None),
            cv: std::sync::Condvar::new(),
        });
        let me = worker.clone();
        std::thread::Builder::new()
            .name("clipboard".into())
            .spawn(move || loop {
                let mut pending = match me.pending.lock() {
                    Ok(g) => g,
                    Err(p) => p.into_inner(),
                };
                while pending.is_none() {
                    pending = match me.cv.wait(pending) {
                        Ok(g) => g,
                        Err(p) => p.into_inner(),
                    };
                }
                let text = pending.take().expect("wait loop guarantees Some");
                drop(pending);
                clipboard_set_text(text);
            })
            .expect("failed to spawn clipboard worker thread");
        worker
    }

    fn submit(&self, text: String) {
        let mut pending = match self.pending.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        *pending = Some(text);
        self.cv.notify_one();
    }
}

/// Queue `text` for the clipboard. The latest submission wins if the worker is
/// still busy writing an earlier one (#4).
fn copy_to_clipboard(text: String) {
    static CLIPBOARD_WORKER: OnceLock<Arc<ClipboardWorker>> = OnceLock::new();
    let worker = CLIPBOARD_WORKER.get_or_init(ClipboardWorker::spawn);
    worker.submit(text);
}

/// JoinHandles of window-scoped background tasks (dialog connection tests,
/// process actions) so closing the window aborts them instead of leaving them
/// to linger on the shared runtime until the process exits (#5).
static WINDOW_BG_TASKS: OnceLock<Mutex<HashMap<u64, Vec<tokio::task::JoinHandle<()>>>>> =
    OnceLock::new();

fn track_bg_task(window_id: u64, handle: tokio::task::JoinHandle<()>) {
    let map = WINDOW_BG_TASKS.get_or_init(Default::default);
    if let Ok(mut map) = map.lock() {
        map.entry(window_id).or_default().push(handle);
    }
}

fn abort_window_bg_tasks(window_id: u64) {
    let Some(map) = WINDOW_BG_TASKS.get() else {
        return;
    };
    if let Ok(mut map) = map.lock() {
        if let Some(tasks) = map.remove(&window_id) {
            for task in tasks {
                task.abort();
            }
        }
    }
}

fn tab_title_len(title: &str) -> i32 {
    title
        .chars()
        .map(|ch| if ch.is_ascii() { 1usize } else { 2usize })
        .sum::<usize>()
        .min(i32::MAX as usize) as i32
}

fn should_block_close(exit_confirmed: bool, has_live_sessions: bool) -> bool {
    !exit_confirmed && has_live_sessions
}

/// Tear down one window's workers (SSH + SFTP) and hide its detachable
/// monitor windows. Idempotent: repeated calls see empty maps. Also aborts
/// every queued auth prompt (host key / credentials / MFA) owned by this
/// window, answering reject/cancel so the blocked connection attempts fail
/// cleanly instead of hanging on a dialog that will never show (#multi-window).
fn teardown_window(
    window_id: u64,
    handles: &Rc<RefCell<HashMap<String, SessionHandle>>>,
    sftp_handles: &SftpHandles,
    proc_weak: &slint::Weak<ProcWindow>,
    sys_weak: &slint::Weak<SystemInfoWindow>,
    editor_weak: &slint::Weak<EditorWindow>,
) {
    abort_window_prompts(window_id);
    abort_window_bg_tasks(window_id);
    {
        let mut sessions = handles.borrow_mut();
        for handle in sessions.values() {
            handle.close();
            // Forcefully cancel the session task so a handshake or reconnect
            // that is still in flight cannot linger until process exit (#5).
            handle.join.abort();
        }
        sessions.clear();
    }
    if let Ok(mut sftp) = sftp_handles.lock() {
        for handle in sftp.values() {
            handle.close();
        }
        sftp.clear();
    }
    if let Some(w) = proc_weak.upgrade() {
        let _ = w.hide();
    }
    if let Some(w) = sys_weak.upgrade() {
        let _ = w.hide();
    }
    if let Some(w) = editor_weak.upgrade() {
        let _ = w.hide();
    }
}

/// Number of samples kept for the sparkline.
const NET_HISTORY_LEN: usize = 60;

// UI-thread handle to the process core, published by `run()` before the
// event loop starts. Cross-thread callers (the single-instance IPC
// listener) run a capture-less closure via `invoke_from_event_loop` and
// fetch the non-Send `Rc<AppCore>` from here instead of moving it across
// threads.
thread_local! {
    static NEW_WINDOW_CORE: RefCell<Option<Rc<AppCore>>> = const { RefCell::new(None) };
}

/// Embed the app icon PNG into the binary and set it as the X11 window icon.
///
/// On X11, the taskbar/dock icon for a running window comes from the
/// `_NET_WM_ICON` property, which winit sets via `Window::set_window_icon`.
/// When the app runs as a bare AppImage (or from a plain directory without
/// running install-linux.sh) there is no installed .desktop + icon, so the
/// dock falls back to a generic gear.  This call fixes that for X11 sessions.
///
/// On Wayland the dock icon is resolved by the compositor from the XDG
/// app-id → .desktop file mapping; `set_window_icon` is a no-op there, so
/// Wayland users still need AppImageLauncher or install-linux.sh for the
/// dock icon.  The `icon:` property in app.slint handles the in-title-bar
/// icon on both backends without any runtime work.
///
/// Windows gets its icon from the `.ico` embedded by winresource at link
/// time; macOS from the app bundle — neither path needs runtime decoding.
pub fn run(_intent: crate::app::launch::LaunchIntent) -> Result<()> {
    // Load the renderer preference before creating any Slint window. Reuse the
    // same store for the rest of the app so startup does not read the config
    // twice merely to select a backend (#280).
    let config = ConfigStore::load().context("failed to load config")?;

    // Windows frameless-window attributes must be fixed before the first Slint
    // window is created; doing it afterwards leaves some Win10 machines with an
    // invisible frame that shifts mouse hit testing (#193).
    #[cfg(windows)]
    setup_windows_platform(config.renderer_mode());

    #[cfg(target_os = "linux")]
    setup_linux_platform(config.renderer_mode());

    // Immersive native title bar on macOS (must precede the first window).
    #[cfg(target_os = "macos")]
    setup_macos_platform(config.renderer_mode());

    // --- Single-instance coordination -------------------------------------
    // All launches for one profile share the same GUI store. Independent
    // process snapshots could otherwise overwrite each other's saved sessions.
    // IPC failure may still fall through; ConfigStore rejects stale writes.
    let si_path = crate::app::single_instance::socket_path();
    let instance = match crate::app::single_instance::acquire(&si_path, true) {
        Ok(i) => Some(i),
        Err(e) => {
            tracing::warn!("single-instance acquire failed: {e}");
            None
        }
    };
    if let Some(crate::app::single_instance::Instance::Forwarded) = instance {
        return Ok(());
    }

    // --- Runtime + store -------------------------------------------------
    let runtime = Arc::new(Runtime::new().context("failed to start tokio runtime")?);
    let store = Rc::new(RefCell::new(config));
    // Reachable from the Slint-thread event handler for recording terminal
    // commands into history (#113).
    HISTORY_STORE.with(|s| *s.borrow_mut() = Some(store.clone()));
    // Debounced config writer for routine UI mutations (#3).
    ConfigWriter::register(store.clone());

    let core = Rc::new(AppCore {
        runtime,
        store,
        registry: Rc::new(WindowRegistry::default()),
        window_states: Rc::new(RefCell::new(HashMap::new())),
        tab_routes: Arc::new(Mutex::new(HashMap::new())),
        first_window_done: Cell::new(false),
    });

    // IPC listener: forwarded "new-window" requests arrive on the listener
    // thread, but open_window() must run on the Slint UI thread — and
    // AppCore holds Rc state, so it cannot be captured by the
    // invoke_from_event_loop closure. The closure therefore captures nothing
    // and fetches the core from a UI-thread-local set just before the event
    // loop starts; until then (early startup) the listener retries briefly so
    // no request is lost.
    if let Some(crate::app::single_instance::Instance::Primary { listen }) = instance {
        std::thread::spawn(move || {
            listen.spawn(move |msg| {
                if msg == "new-window" {
                    tracing::info!("single-instance: new-window request received");
                    // invoke_from_event_loop fails forever once the event
                    // loop is gone (app quitting) — retry only briefly so
                    // this listener callback cannot spin indefinitely.
                    let deadline =
                        std::time::Instant::now() + std::time::Duration::from_secs(10);
                    while slint::invoke_from_event_loop(|| {
                        NEW_WINDOW_CORE.with(|c| {
                            if let Some(core) = c.borrow().clone() {
                                match open_window(core.clone(), true, None) {
                                    Ok(window_id) => {
                                        // The request came from an OS entry
                                        // point while we may be in the
                                        // background — bring the new window
                                        // to the front (best effort).
                                        if let Some(st) =
                                            core.window_states.borrow().get(&window_id)
                                        {
                                            if let Some(w) = st.weak.upgrade() {
                                                raise_to_front(&w);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        tracing::warn!("failed to open forwarded window: {e:#}")
                                    }
                                }
                            }
                        });
                    })
                    .is_err()
                    {
                        if std::time::Instant::now() >= deadline {
                            tracing::warn!(
                                "single-instance: event loop unreachable, dropping new-window request"
                            );
                            break;
                        }
                        // The event loop provider appears after startup begins;
                        // retry instead of dropping an explicit user action.
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                }
            });
        });
    }

    // Set the Wayland app_id / X11 WM_CLASS *before* the window is created so
    // the Linux desktop shell can match the running window to the installed
    // `meatshell.desktop` entry and show our icon in the dock/taskbar.  (On
    // Windows the icon comes from the embedded .ico, so this is a no-op there.)
    let _ = slint::set_xdg_app_id("meatshell");

    // Taskbar jump list ("新建窗口") on Windows: register before the first
    // window shows so the entry is available immediately. Failure is
    // warn-only and never blocks startup.
    #[cfg(windows)]
    crate::app::jump_list::register_new_window_task();

    open_window(core.clone(), false, None)?;
    let tray_timer = tray::install(core.clone());

    // Publish the core to the UI thread so the IPC listener's
    // invoke_from_event_loop closures can open windows without capturing the
    // non-Send Rc<AppCore> (see the listener above).
    NEW_WINDOW_CORE.with(|c| *c.borrow_mut() = Some(core.clone()));

    // Global loop: window.run() returns when *its* window closes, which is
    // wrong once several windows share the loop.
    let loop_result = slint::run_event_loop();
    if let Err(e) = &loop_result {
        tracing::warn!("event loop exited with error ({e:#}); running bounded shutdown anyway");
    }
    // Bound the shutdown instead of relying on Rust's drop glue: tokio's
    // Runtime Drop waits indefinitely for tasks that never finished
    // (telnet/local/sftp pumps, spawn_blocking helpers) — the observed
    // windowless lingering process on Windows 11. Take ownership of the
    // runtime when all holders have gone, give stragglers two seconds, then
    // hard-exit unconditionally. The event-loop error path goes through here
    // too: propagating with `?` would hand the runtime to the TLS destructor
    // chain, where a wedged blocking thread could hang the process forever.
    NEW_WINDOW_CORE.with(|c| *c.borrow_mut() = None);
    tray::clear();
    drop(tray_timer);
    if let Ok(core_owned) = Rc::try_unwrap(core) {
        if let Ok(runtime) = Arc::try_unwrap(core_owned.runtime) {
            runtime.shutdown_timeout(std::time::Duration::from_secs(2));
        }
    }
    std::process::exit(0);
}

#[cfg(test)]
#[path = "../tests/app/welcome_sidebar/mod.rs"]
mod welcome_sidebar_tests;

#[cfg(test)]
#[path = "../tests/app/terminal_input/mod.rs"]
mod key_tests;

#[cfg(test)]
#[path = "../tests/app/terminal_rendering/mod.rs"]
mod selection_tests;

#[cfg(test)]
#[path = "../tests/app/output_highlighting/mod.rs"]
mod log_highlight_tests;

#[cfg(test)]
#[path = "../tests/app/text_editor/mod.rs"]
mod text_editor_tests;

#[cfg(test)]
#[path = "../tests/app/modal_layers.rs"]
mod modal_layers_tests;
