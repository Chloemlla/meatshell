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
