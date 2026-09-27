use super::*;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Key, PointerEventButton, WindowEvent};

struct Backend(Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>);
impl slint::platform::Platform for Backend {
    fn create_window_adapter(
        &self,
    ) -> Result<Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        self.0.borrow_mut().push(window.clone());
        Ok(window)
    }
}

fn settle(window: &MinimalSoftwareWindow) {
    for _ in 0..2 {
        slint::platform::update_timers_and_animations();
        let mut pixels = vec![slint::Rgb8Pixel::default(); 1000 * 850];
        window.request_redraw();
        window.draw_if_needed(|r| {
            r.render(&mut pixels, 1000);
        });
    }
    slint::platform::update_timers_and_animations();
}

fn key(window: &MinimalSoftwareWindow, text: impl Into<SharedString>) {
    let text = text.into();
    window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window.dispatch_event(WindowEvent::KeyReleased { text });
}

fn host() -> (
    AppWindow,
    Rc<MinimalSoftwareWindow>,
    Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>,
) {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Backend(windows.clone()))).unwrap();
    let ui = AppWindow::new().unwrap();
    ui.show().unwrap();
    let window = windows.borrow()[0].clone();
    window.set_size(slint::PhysicalSize::new(1000, 850));
    settle(&window);
    (ui, window, windows)
}

fn bind_cancellations(ui: &AppWindow, calls: Rc<RefCell<Vec<&'static str>>>, window_id: u64) {
    let weak = ui.as_weak();
    let count = calls.clone();
    ui.on_session_dialog_cancel(move || {
        count.borrow_mut().push("session");
        weak.upgrade().unwrap().set_dialog_open(false);
    });
    let weak = ui.as_weak();
    let count = calls.clone();
    ui.on_cred_reject(move || {
        count.borrow_mut().push("credentials");
        resolve_front_cred(&weak.upgrade().unwrap(), window_id, false);
    });
    let weak = ui.as_weak();
    let count = calls.clone();
    ui.on_mfa_cancel(move || {
        count.borrow_mut().push("mfa");
        resolve_front_mfa(&weak.upgrade().unwrap(), window_id, false);
    });
    let weak = ui.as_weak();
    let count = calls.clone();
    ui.on_hostkey_reject(move || {
        count.borrow_mut().push("host-key");
        resolve_front_hostkey(&weak.upgrade().unwrap(), window_id, false);
    });
    ui.on_paste_confirm_cancelled(move || {
        calls.borrow_mut().push("paste");
    });
}

#[test]
fn every_app_modal_cancels_after_keyboard_navigation() {
    std::thread::spawn(|| {
        let (ui, window, _) = host();
        let calls = Rc::new(RefCell::new(Vec::new()));
        bind_cancellations(&ui, calls.clone(), 10);
        type Case = (&'static str, fn(&AppWindow, bool), fn(&AppWindow) -> bool);
        let cases: &[Case] = &[
            (
                "shortcuts",
                AppWindow::set_shortcuts_open,
                AppWindow::get_shortcuts_open,
            ),
            (
                "about",
                AppWindow::set_about_open,
                AppWindow::get_about_open,
            ),
            (
                "settings",
                AppWindow::set_interface_open,
                AppWindow::get_interface_open,
            ),
            (
                "session",
                AppWindow::set_dialog_open,
                AppWindow::get_dialog_open,
            ),
            (
                "group",
                AppWindow::set_group_dialog_open,
                AppWindow::get_group_dialog_open,
            ),
            (
                "rename",
                AppWindow::set_tab_rename_open,
                AppWindow::get_tab_rename_open,
            ),
            (
                "commands",
                AppWindow::set_quick_cmd_manage_open,
                AppWindow::get_quick_cmd_manage_open,
            ),
            (
                "command-group",
                AppWindow::set_qcg_open,
                AppWindow::get_qcg_open,
            ),
            (
                "sftp",
                AppWindow::set_sftp_prompt_open,
                AppWindow::get_sftp_prompt_open,
            ),
            (
                "permissions",
                AppWindow::set_chmod_open,
                AppWindow::get_chmod_open,
            ),
            (
                "import",
                AppWindow::set_batch_import_open,
                AppWindow::get_batch_import_open,
            ),
            (
                "delete",
                AppWindow::set_confirm_delete_open,
                AppWindow::get_confirm_delete_open,
            ),
            (
                "paste",
                AppWindow::set_paste_confirm_open,
                AppWindow::get_paste_confirm_open,
            ),
            (
                "host-key",
                AppWindow::set_hostkey_prompt_open,
                AppWindow::get_hostkey_prompt_open,
            ),
            (
                "credentials",
                AppWindow::set_cred_prompt_open,
                AppWindow::get_cred_prompt_open,
            ),
            (
                "mfa",
                AppWindow::set_mfa_prompt_open,
                AppWindow::get_mfa_prompt_open,
            ),
            (
                "exit",
                AppWindow::set_confirm_close_open,
                AppWindow::get_confirm_close_open,
            ),
        ];
        for (name, open, is_open) in cases {
            open(&ui, true);
            settle(&window);
            assert!(ui.get_modal_open(), "{name} must own the modal layer");
            for _ in 0..32 {
                key(&window, Key::Tab);
            }
            key(&window, Key::Escape);
            settle(&window);
            assert!(
                !is_open(&ui),
                "{name}: Escape was lost after Tab navigation"
            );
            assert!(!ui.get_modal_open());
            ui.set_is_mac(true);
            open(&ui, true);
            settle(&window);
            for _ in 0..32 {
                key(&window, Key::Backtab);
            }
            let command = if cfg!(target_os = "macos") {
                Key::Meta
            } else {
                Key::Control
            };
            window.dispatch_event(WindowEvent::KeyPressed {
                text: command.into(),
            });
            key(&window, ".");
            window.dispatch_event(WindowEvent::KeyReleased {
                text: command.into(),
            });
            ui.set_is_mac(false);
            settle(&window);
            assert!(
                !is_open(&ui),
                "{name}: reverse navigation escaped the modal"
            );
        }
        for expected in ["session", "host-key", "credentials", "mfa", "paste"] {
            assert_eq!(calls.borrow().iter().filter(|&&s| s == expected).count(), 2);
        }
    })
    .join()
    .unwrap();
}

#[test]
fn top_layer_keeps_underlying_draft_and_uses_business_cancellation() {
    std::thread::spawn(|| {
        let (ui, window, _) = host();
        let calls = Rc::new(RefCell::new(Vec::new()));
        bind_cancellations(&ui, calls.clone(), 20);
        ui.set_dialog_open(true);
        ui.set_dialog_password("unsaved-fixture".into());
        settle(&window);
        ui.set_cred_password("typed-fixture".into());
        ui.set_cred_prompt_open(true);
        settle(&window);
        key(&window, Key::Escape);
        settle(&window);
        assert!(!ui.get_cred_prompt_open());
        assert!(ui.get_cred_password().is_empty());
        assert!(ui.get_dialog_open());
        assert_eq!(ui.get_dialog_password(), "unsaved-fixture");
        assert_eq!(&*calls.borrow(), &["credentials"]);
        ui.set_confirm_close_open(true);
        settle(&window);
        key(&window, Key::Escape);
        settle(&window);
        assert!(!ui.get_confirm_close_open());
        assert!(ui.get_dialog_open());
        key(&window, Key::Escape);
        settle(&window);
        assert!(!ui.get_dialog_open());
        assert!(ui.get_dialog_password().is_empty());
        assert_eq!(&*calls.borrow(), &["credentials", "session"]);
    })
    .join()
    .unwrap();
}

#[test]
fn confirmation_and_backdrop_policies_are_explicit() {
    std::thread::spawn(|| {
        let (ui, window, _) = host();
        let confirms = Rc::new(Cell::new(0));
        let count = confirms.clone();
        ui.on_paste_confirmed(move |_| count.set(count.get() + 1));
        ui.set_confirm_delete_open(true);
        settle(&window);
        key(&window, Key::Return);
        assert!(
            ui.get_confirm_delete_open(),
            "destructive Enter must remain opt-in"
        );
        key(&window, Key::Escape);
        settle(&window);
        ui.set_paste_confirm_open(true);
        settle(&window);
        let pos = slint::LogicalPosition::new(8., 8.);
        window.dispatch_event(WindowEvent::PointerPressed {
            position: pos,
            button: PointerEventButton::Left,
        });
        window.dispatch_event(WindowEvent::PointerReleased {
            position: pos,
            button: PointerEventButton::Left,
        });
        assert!(
            ui.get_paste_confirm_open(),
            "protected backdrop must not cancel paste"
        );
        key(&window, Key::Return);
        settle(&window);
        assert!(!ui.get_paste_confirm_open());
        assert_eq!(confirms.get(), 1);
    })
    .join()
    .unwrap();
}

#[test]
fn modal_keyboard_state_is_local_to_each_window() {
    std::thread::spawn(|| {
        let (first, first_window, windows) = host();
        let second = AppWindow::new().unwrap();
        second.show().unwrap();
        let second_window = windows.borrow()[1].clone();
        second_window.set_size(slint::PhysicalSize::new(1000, 850));
        first.set_shortcuts_open(true);
        second.set_about_open(true);
        settle(&first_window);
        settle(&second_window);
        key(&second_window, Key::Escape);
        settle(&second_window);
        assert!(!second.get_about_open());
        assert!(first.get_shortcuts_open());
        key(&first_window, Key::Escape);
        settle(&first_window);
        assert!(!first.get_shortcuts_open());
    })
    .join()
    .unwrap();
}

#[test]
fn detached_windows_share_cancel_keys_and_editor_guards_unsaved_changes() {
    std::thread::spawn(|| {
        let (_main, _, windows) = host();
        let process_ui = ProcWindow::new().unwrap();
        process_ui.show().unwrap();
        let process_window = windows.borrow()[1].clone();
        process_window.set_size(slint::PhysicalSize::new(1000, 850));
        let closes = Rc::new(Cell::new(0));
        let count = closes.clone();
        process_ui.on_close(move || count.set(count.get() + 1));
        settle(&process_window);
        key(&process_window, Key::Escape);
        assert_eq!(closes.get(), 1);

        let system_ui = SystemInfoWindow::new().unwrap();
        system_ui.show().unwrap();
        let system_window = windows.borrow()[2].clone();
        system_window.set_size(slint::PhysicalSize::new(1000, 850));
        let count = closes.clone();
        system_ui.on_close(move || count.set(count.get() + 1));
        settle(&system_window);
        key(&system_window, Key::Escape);
        assert_eq!(closes.get(), 2);

        let editor = EditorWindow::new().unwrap();
        editor.set_editor_open(true);
        editor.show().unwrap();
        let editor_window = windows.borrow()[3].clone();
        editor_window.set_size(slint::PhysicalSize::new(1000, 850));
        let count = closes.clone();
        editor.on_close_editor(move || count.set(count.get() + 1));
        settle(&editor_window);
        key(&editor_window, "x");
        assert_eq!(
            editor.get_editor_content(),
            "x",
            "editor must restore input focus"
        );
        assert!(editor.get_editor_dirty());
        key(&editor_window, Key::Escape);
        settle(&editor_window);
        assert!(editor.get_discard_confirm_open());
        assert_eq!(closes.get(), 2);
        key(&editor_window, Key::Return);
        assert!(
            editor.get_discard_confirm_open(),
            "Enter must not discard changes"
        );
        key(&editor_window, Key::Escape);
        settle(&editor_window);
        assert!(!editor.get_discard_confirm_open());
        assert_eq!(editor.get_editor_content(), "x");
        editor.set_editor_dirty(false);
        key(&editor_window, Key::Escape);
        assert_eq!(closes.get(), 3);
    })
    .join()
    .unwrap();
}
