//! Built-in file viewer/editor hosts (#70, #494).
//!
//! A text file opens either in the detached `EditorWindow` or in the panel
//! docked inside the main `AppWindow` (Settings → SFTP). Both expose the same
//! `editor-*` properties and callbacks, so the open/save/find/close logic is
//! written once against [`EditorHost`] and wired to each host.

use super::*;

pub(super) trait EditorHost: slint::ComponentHandle + 'static {
    fn set_editor_open(&self, open: bool);
    fn set_editor_name(&self, name: SharedString);
    fn get_editor_path(&self) -> SharedString;
    fn set_editor_path(&self, path: SharedString);
    fn get_editor_tab_id(&self) -> SharedString;
    fn set_editor_tab_id(&self, tab_id: SharedString);
    fn get_editor_content(&self) -> SharedString;
    fn set_editor_content(&self, content: SharedString);
    fn get_editor_readonly(&self) -> bool;
    fn set_editor_readonly(&self, readonly: bool);
    fn set_editor_dirty(&self, dirty: bool);
    fn set_editor_language(&self, language: SharedString);
    fn set_editor_syntax_spans(&self, spans: SharedString);
    fn set_editor_syntax_source(&self, source: SharedString);
    fn set_editor_find_query(&self, query: SharedString);
    fn set_editor_replace_text(&self, text: SharedString);
    fn set_editor_match_count(&self, count: i32);
    fn set_editor_find_position(&self, position: i32);
    fn get_dark_mode(&self) -> bool;

    fn on_save_file(&self, f: impl FnMut() + 'static);
    fn on_close_editor(&self, f: impl FnMut() + 'static);
    fn on_editor_highlight(&self, f: impl FnMut(SharedString) + 'static);
    fn on_editor_count_matches(&self, f: impl FnMut(SharedString, SharedString) -> i32 + 'static);
    fn on_editor_find_step(
        &self,
        f: impl FnMut(SharedString, SharedString, i32, bool) -> EditorFindResult + 'static,
    );
    fn on_editor_replace_all(&self, f: impl FnMut(SharedString, SharedString) + 'static);

    /// Bring the editor on screen after a document was loaded.
    fn present_editor(&self);
    /// Take the editor off screen after it was closed.
    fn dismiss_editor(&self);
}

// Inherent methods win over trait methods in path resolution, so these
// forward to the Slint-generated accessors rather than recursing.
macro_rules! forward_editor_host {
    ($ty:ty) => {
        fn set_editor_open(&self, v: bool) {
            <$ty>::set_editor_open(self, v)
        }
        fn set_editor_name(&self, v: SharedString) {
            <$ty>::set_editor_name(self, v)
        }
        fn get_editor_path(&self) -> SharedString {
            <$ty>::get_editor_path(self)
        }
        fn set_editor_path(&self, v: SharedString) {
            <$ty>::set_editor_path(self, v)
        }
        fn get_editor_tab_id(&self) -> SharedString {
            <$ty>::get_editor_tab_id(self)
        }
        fn set_editor_tab_id(&self, v: SharedString) {
            <$ty>::set_editor_tab_id(self, v)
        }
        fn get_editor_content(&self) -> SharedString {
            <$ty>::get_editor_content(self)
        }
        fn set_editor_content(&self, v: SharedString) {
            <$ty>::set_editor_content(self, v)
        }
        fn get_editor_readonly(&self) -> bool {
            <$ty>::get_editor_readonly(self)
        }
        fn set_editor_readonly(&self, v: bool) {
            <$ty>::set_editor_readonly(self, v)
        }
        fn set_editor_dirty(&self, v: bool) {
            <$ty>::set_editor_dirty(self, v)
        }
        fn set_editor_language(&self, v: SharedString) {
            <$ty>::set_editor_language(self, v)
        }
        fn set_editor_syntax_spans(&self, v: SharedString) {
            <$ty>::set_editor_syntax_spans(self, v)
        }
        fn set_editor_syntax_source(&self, v: SharedString) {
            <$ty>::set_editor_syntax_source(self, v)
        }
        fn set_editor_find_query(&self, v: SharedString) {
            <$ty>::set_editor_find_query(self, v)
        }
        fn set_editor_replace_text(&self, v: SharedString) {
            <$ty>::set_editor_replace_text(self, v)
        }
        fn set_editor_match_count(&self, v: i32) {
            <$ty>::set_editor_match_count(self, v)
        }
        fn set_editor_find_position(&self, v: i32) {
            <$ty>::set_editor_find_position(self, v)
        }
        fn get_dark_mode(&self) -> bool {
            <$ty>::get_dark_mode(self)
        }

        fn on_save_file(&self, f: impl FnMut() + 'static) {
            <$ty>::on_save_file(self, f)
        }
        fn on_close_editor(&self, f: impl FnMut() + 'static) {
            <$ty>::on_close_editor(self, f)
        }
        fn on_editor_highlight(&self, f: impl FnMut(SharedString) + 'static) {
            <$ty>::on_editor_highlight(self, f)
        }
        fn on_editor_count_matches(
            &self,
            f: impl FnMut(SharedString, SharedString) -> i32 + 'static,
        ) {
            <$ty>::on_editor_count_matches(self, f)
        }
        fn on_editor_find_step(
            &self,
            f: impl FnMut(SharedString, SharedString, i32, bool) -> EditorFindResult + 'static,
        ) {
            <$ty>::on_editor_find_step(self, f)
        }
        fn on_editor_replace_all(&self, f: impl FnMut(SharedString, SharedString) + 'static) {
            <$ty>::on_editor_replace_all(self, f)
        }
    };
}

impl EditorHost for EditorWindow {
    forward_editor_host!(EditorWindow);

    fn present_editor(&self) {
        let _ = self.show();
        self.window().with_winit_window(|ww| ww.focus_window());
    }

    fn dismiss_editor(&self) {
        let _ = self.hide();
    }
}

impl EditorHost for AppWindow {
    forward_editor_host!(AppWindow);

    // The docked panel appears and disappears with `editor-open`; it takes
    // keyboard focus itself once created.
    fn present_editor(&self) {}

    fn dismiss_editor(&self) {
        self.set_editor_discard_confirm_open(false);
    }
}

/// Load a remote text file into `host` and show it.
pub(super) fn open_document<H: EditorHost>(
    host: &H,
    tab_id: &str,
    path: String,
    name: String,
    content: String,
    edit: bool,
) {
    host.set_editor_tab_id(tab_id.into());
    host.set_editor_path(path.into());
    host.set_editor_name(name.into());
    editor_syntax::refresh(host, &content);
    host.set_editor_content(content.into());
    host.set_editor_readonly(!edit);
    host.set_editor_dirty(false);
    host.set_editor_open(true);
    host.present_editor();
}

/// Save, close, syntax refresh and find/replace for one editor host.
pub(super) fn wire_editor_host<H: EditorHost>(host: &H, sftp_handles: SftpHandles) {
    // Refresh syntax after edits. The debounced callback keeps typing
    // responsive while the native TextInput owns selection and IME handling.
    {
        let weak = host.as_weak();
        host.on_editor_highlight(move |text: SharedString| {
            if let Some(h) = weak.upgrade() {
                editor_syntax::refresh(&h, text.as_str());
            }
        });
    }

    // Save (Ctrl+S / button) writes the text back to the remote file (#70).
    // Read-only (view) sessions never save.
    {
        let weak = host.as_weak();
        host.on_save_file(move || {
            let Some(h) = weak.upgrade() else { return };
            if h.get_editor_readonly() {
                return;
            }
            let path = h.get_editor_path().to_string();
            let content = h.get_editor_content().to_string();
            let tab_id = h.get_editor_tab_id().to_string();
            if let Ok(handles) = sftp_handles.lock() {
                if let Some(sftp) = handles.get(&tab_id) {
                    sftp.write_text(path, content);
                }
            }
            h.set_editor_dirty(false);
        });
    }

    // Closing discards unsaved edits. Saving is an explicit action; silently
    // uploading on X or Esc can overwrite a remote file the user only meant to
    // inspect (#287). The UI asks before discarding unsaved changes.
    {
        let weak = host.as_weak();
        host.on_close_editor(move || {
            let Some(h) = weak.upgrade() else { return };
            h.set_editor_open(false);
            h.dismiss_editor();
            h.set_editor_dirty(false);
            h.set_editor_find_query("".into());
            h.set_editor_replace_text("".into());
            h.set_editor_match_count(0);
            h.set_editor_find_position(-1);
        });
    }

    // Find/replace (#287). Matching is literal and case-sensitive, which is
    // predictable for configuration/source files.
    host.on_editor_count_matches(|content: SharedString, query: SharedString| {
        count_matches(content.as_str(), query.as_str())
    });
    host.on_editor_find_step(
        |content: SharedString, query: SharedString, current: i32, reverse: bool| {
            find_step(content.as_str(), query.as_str(), current, reverse)
        },
    );
    {
        let weak = host.as_weak();
        host.on_editor_replace_all(move |query: SharedString, replacement: SharedString| {
            let Some(h) = weak.upgrade() else { return };
            if h.get_editor_readonly() || query.is_empty() {
                return;
            }
            let replaced = h
                .get_editor_content()
                .replace(query.as_str(), replacement.as_str());
            editor_syntax::refresh(&h, &replaced);
            h.set_editor_content(replaced.into());
            h.set_editor_dirty(true);
            h.set_editor_match_count(0);
        });
    }
}

fn count_matches(content: &str, query: &str) -> i32 {
    if query.is_empty() {
        0
    } else {
        content.matches(query).count().min(i32::MAX as usize) as i32
    }
}

fn find_step(content: &str, query: &str, current: i32, reverse: bool) -> EditorFindResult {
    if query.is_empty() {
        return EditorFindResult {
            start: 0,
            end: 0,
            count: 0,
        };
    }
    let positions: Vec<usize> = content
        .match_indices(query)
        .map(|(index, _)| index)
        .collect();
    let selected = if reverse {
        positions
            .iter()
            .rev()
            .copied()
            .find(|position| current < 0 || *position < current as usize)
            .or_else(|| positions.last().copied())
    } else {
        positions
            .iter()
            .copied()
            .find(|position| current < 0 || *position > current as usize)
            .or_else(|| positions.first().copied())
    };
    let start = selected.unwrap_or(0).min(i32::MAX as usize) as i32;
    EditorFindResult {
        start,
        end: start.saturating_add(query.len().min(i32::MAX as usize) as i32),
        count: positions.len().min(i32::MAX as usize) as i32,
    }
}
