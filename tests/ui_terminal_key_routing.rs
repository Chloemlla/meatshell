#[test]
fn scrollback_navigation_is_consumed_only_when_rust_handles_it() {
    let source = include_str!("../ui/terminal_view.slint");
    let branch_start = source
        .find("Local history navigation")
        .expect("terminal history navigation branch");
    let branch_end = source[branch_start..]
        .find("// ── All other keys")
        .expect("PTY fallback branch")
        + branch_start;
    let branch = &source[branch_start..branch_end];

    assert!(branch.contains("event.text == Key.Home"));
    assert!(branch.contains("event.text == Key.End"));
    assert!(branch.contains("event.text == Key.PageUp"));
    assert!(branch.contains("event.text == Key.PageDown"));
    assert!(branch.contains("&& root.terminal-scrollback-key(event.text)"));
    assert!(source[branch_end..].contains("root.send-key(event.text"));

    let app_source = include_str!("../ui/app.slint");
    assert!(app_source.contains(
        "callback terminal-scrollback-key(string /* tab-id */, string /* key */) -> bool;"
    ));
    assert!(app_source.contains(
        "terminal-scrollback-key(key) => { root.terminal-scrollback-key(term.id, key) }"
    ));
}

#[test]
fn scrollback_transition_reclaims_the_hidden_ime_anchor_on_the_next_ui_turn() {
    let source = include_str!("../ui/terminal_view.slint");
    let transition_start = source
        .find("changed cursor-row => {")
        .expect("scrollback cursor transition handler");
    let transition = &source[transition_start..];

    assert!(transition.contains("if (root.cursor-row < 0) {"));
    assert!(transition.contains("root.focus-pending = true;"));
    assert!(source.contains("focus-defer-timer := Timer"));
    assert!(source.contains("ime-input.focus();"));
}

#[test]
fn ctrl_v_pastes_locally_except_on_the_alternate_screen() {
    let source = include_str!("../ui/terminal_view.slint");
    let paste_start = source.find("// ── Paste:").expect("paste key routing");
    let paste_end = source[paste_start..]
        .find("// ── Paste: Shift+Insert")
        .expect("Shift+Insert routing")
        + paste_start;
    let paste = &source[paste_start..paste_end];

    assert!(paste.contains("event.modifiers.control && event.modifiers.shift"));
    assert!(paste.contains("!root.is-alt-screen"));
    assert!(paste.contains("!event.modifiers.shift"));
    assert!(source[paste_end..].contains("root.send-key(event.text"));
}
