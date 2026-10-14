//! Connects the shared update state to each window's existing update controls.

use super::*;
use crate::update::Release;

#[derive(Default)]
pub struct State {
    release: Option<Release>,
    busy: bool,
    status: String,
    error: bool,
    #[cfg(windows)]
    download: Option<crate::update::windows::Download>,
}

fn publish(core: &Rc<AppCore>) {
    let state = core.update.borrow();
    core.registry.for_each(|weak| {
        if let Some(window) = weak.upgrade() {
            window.set_update_busy(state.busy);
            window.set_update_status(state.status.clone().into());
            window.set_update_error(state.error);
            window.set_update_version(
                state
                    .release
                    .as_ref()
                    .map(|r| r.tag_name.as_str())
                    .unwrap_or("")
                    .into(),
            );
            window.set_update_available(state.release.is_some());
            #[cfg(windows)]
            window.set_update_ready(state.download.is_some());
        }
    });
}

fn on_ui(f: impl FnOnce(&Rc<AppCore>) + Send + 'static) {
    let _ = slint::invoke_from_event_loop(move || {
        NEW_WINDOW_CORE.with(|slot| {
            if let Some(core) = slot.borrow().as_ref() {
                f(core);
            }
        });
    });
}

fn check(core: &Rc<AppCore>, manual: bool) {
    if core.update.borrow().busy {
        return;
    }
    {
        let mut state = core.update.borrow_mut();
        state.busy = true;
        state.error = false;
        state.status = t("正在检查更新…", "Checking for updates…").into();
    }
    publish(core);
    std::thread::spawn(move || {
        let result = crate::update::check();
        on_ui(move |core| {
            {
                let mut state = core.update.borrow_mut();
                state.busy = false;
                match result {
                    Ok(release) => {
                        #[cfg(windows)]
                        if state.release.as_ref().map(|r| &r.tag_name)
                            != release.as_ref().map(|r| &r.tag_name)
                        {
                            state.download = None;
                        }
                        state.status = if release.is_some() {
                            t("有新版本可用", "An update is available")
                        } else {
                            t("当前已是最新版本", "You are up to date")
                        }
                        .into();
                        state.release = release;
                    }
                    Err(error) => {
                        state.error = manual;
                        state.status = if manual {
                            format!(
                                "{}: {error:#}",
                                t("检查失败，请重试", "Update check failed; please retry")
                            )
                        } else {
                            String::new()
                        };
                    }
                }
            }
            publish(core);
        });
    });
}

#[cfg(windows)]
fn download(core: &Rc<AppCore>) {
    if core.update.borrow().busy {
        return;
    }
    let Some(release) = core.update.borrow().release.clone() else {
        return;
    };
    {
        let mut state = core.update.borrow_mut();
        state.busy = true;
        state.error = false;
        state.status = t("正在下载更新…", "Downloading update…").into();
    }
    publish(core);
    std::thread::spawn(move || {
        let mut last_percent = 0;
        let result = release.windows_installer().and_then(|asset| {
            crate::update::windows::Download::fetch(asset, |received, size| {
                let percent = (received.saturating_mul(100) / size) as i32;
                if percent != last_percent {
                    last_percent = percent;
                    on_ui(move |core| {
                        core.update.borrow_mut().status =
                            format!("{} {percent}%", t("正在下载", "Downloading"));
                        publish(core);
                    });
                }
            })
        });
        on_ui(move |core| {
            {
                let mut state = core.update.borrow_mut();
                state.busy = false;
                match result {
                    Ok(package) => {
                        state.download = Some(package);
                        state.status = t(
                            "下载已验证，可以安装",
                            "Download verified; ready to install",
                        )
                        .into();
                    }
                    Err(error) => {
                        state.error = true;
                        state.status = format!(
                            "{}: {error:#}",
                            t("下载失败，请重试", "Download failed; please retry")
                        );
                    }
                }
            }
            publish(core);
        });
    });
}

#[cfg(windows)]
fn install(core: &Rc<AppCore>, weak: &slint::Weak<AppWindow>) {
    if core.update.borrow().busy {
        return;
    }
    let Some(executable) = crate::update::windows::installed_executable() else {
        return;
    };
    let result = DialogOwner::of_weak(weak).message()
        .set_title(t("安装更新", "Install update"))
        .set_description(t(
            "将关闭所有 MeatShell 窗口和连接，然后安装更新并重新打开程序。请先保存编辑中的文件。Windows 可能要求管理员授权。",
            "All MeatShell windows and connections will close. The update will be installed and the app reopened. Save any files you are editing first. Windows may request administrator permission.",
        ))
        .set_buttons(rfd::MessageButtons::OkCancel).show();
    if result != rfd::MessageDialogResult::Ok {
        return;
    }
    let Some(mut package) = core.update.borrow_mut().download.take() else {
        return;
    };
    let data_dir = crate::config::data_dir();
    let english = crate::i18n::is_en();
    {
        let mut state = core.update.borrow_mut();
        state.busy = true;
        state.status = t("正在准备安装…", "Preparing installation…").into();
        state.error = false;
    }
    publish(core);
    std::thread::spawn(move || {
        let result = package.launch(&executable, &data_dir, english);
        on_ui(move |core| match result {
            Ok(()) => {
                // Use the same shutdown path as Quit: save layouts and stop workers.
                let windows: Vec<_> = core
                    .window_states
                    .borrow()
                    .values()
                    .map(|state| state.main_win.clone())
                    .collect();
                for window in windows {
                    window.invoke_confirm_close_yes();
                }
            }
            Err(error) => {
                drop(package);
                {
                    let mut state = core.update.borrow_mut();
                    state.busy = false;
                    state.error = true;
                    state.status = format!(
                        "{}: {error:#}",
                        t(
                            "无法开始安装，请重新下载",
                            "Could not start installation; download again"
                        )
                    );
                }
                publish(core);
            }
        });
    });
}

pub(super) fn wire(core: &Rc<AppCore>, window: &AppWindow) {
    #[cfg(windows)]
    window.set_update_install_supported(crate::update::windows::installed_executable().is_some());
    {
        let core = core.clone();
        window.on_check_for_updates(move || check(&core, true));
    }
    {
        let core = core.clone();
        let weak = window.as_weak();
        window.on_update_action(move || {
            let Some(window) = weak.upgrade() else { return };
            #[cfg(windows)]
            if window.get_update_install_supported() {
                if core.update.borrow().download.is_some() {
                    install(&core, &weak);
                } else {
                    download(&core);
                }
                return;
            }
            window.invoke_open_update_url();
        });
    }
    publish(core);
    let first_window = !core.first_window_done.replace(true);
    if first_window && core.store.borrow().update_check_enabled() {
        check(core, false);
    }
}

/// The application uses process::exit after bounded shutdown, so drop pending
/// downloads explicitly rather than relying on destructors at process exit.
pub(super) fn cleanup(core: &Rc<AppCore>) {
    #[cfg(windows)]
    {
        core.update.borrow_mut().download = None;
    }
}
