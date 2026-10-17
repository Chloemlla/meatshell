//! One-click launchers for coding-agent CLIs (#427).
//!
//! Like Warp's new-tab menu: the "⌄" next to each pane's "+" lists the local
//! shells and every known agent CLI found on this machine. Picking an agent
//! opens a local terminal tab (PowerShell on Windows, the login shell
//! elsewhere) that starts it; the shell stays open after the agent exits.
//! The same agents are listed in a read-only "agents" group in Quick connect.
//! Detection only looks for executables; nothing is installed or configured.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::config::Session;

use slint::ComponentHandle as _;

use super::session_models::builtin_local_session;

/// Session-id prefix for agent launchers, next to the `system:` built-ins.
pub(super) const AGENT_ID_PREFIX: &str = "agent:";

/// Built-in Quick connect group holding the detected agents. Like "system" it
/// is generated, read-only (no edit / move / delete) and not saved to config.
pub(super) const AGENTS_GROUP: &str = "agents";

/// Result of the first detection pass, shared by every window.
static DETECTED: OnceLock<Vec<DetectedAgent>> = OnceLock::new();

pub(super) struct AgentSpec {
    /// Stable id suffix (`agent:<key>`).
    pub key: &'static str,
    /// Button and tab title.
    pub name: &'static str,
    /// Executable name without extension.
    pub program: &'static str,
}

/// Known agent CLIs, in button order.
pub(super) const AGENTS: &[AgentSpec] = &[
    AgentSpec {
        key: "claude",
        name: "Claude Code",
        program: "claude",
    },
    AgentSpec {
        key: "codex",
        name: "Codex",
        program: "codex",
    },
    AgentSpec {
        key: "gemini",
        name: "Gemini CLI",
        program: "gemini",
    },
    AgentSpec {
        key: "qwen",
        name: "Qwen Code",
        program: "qwen",
    },
    AgentSpec {
        key: "copilot",
        name: "Copilot CLI",
        program: "copilot",
    },
    AgentSpec {
        key: "opencode",
        name: "OpenCode",
        program: "opencode",
    },
    AgentSpec {
        key: "kimi",
        name: "Kimi CLI",
        program: "kimi",
    },
    AgentSpec {
        key: "cursor",
        name: "Cursor CLI",
        program: "cursor-agent",
    },
    AgentSpec {
        key: "codebuddy",
        name: "CodeBuddy",
        program: "codebuddy",
    },
    AgentSpec {
        key: "iflow",
        name: "iFlow CLI",
        program: "iflow",
    },
    AgentSpec {
        key: "crush",
        name: "Crush",
        program: "crush",
    },
    AgentSpec {
        key: "aider",
        name: "Aider",
        program: "aider",
    },
];

/// Extensions tried in order on Windows. A native `.exe` (Claude Code's own
/// installer) wins over npm's `.cmd` shim; `.ps1` shims are skipped because
/// the default execution policy refuses to run them.
#[cfg(windows)]
const EXTENSIONS: &[&str] = &[".exe", ".com", ".cmd", ".bat"];
#[cfg(not(windows))]
const EXTENSIONS: &[&str] = &[""];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DetectedAgent {
    pub key: &'static str,
    pub name: &'static str,
    pub path: PathBuf,
}

/// Every known agent installed on this machine, in `AGENTS` order.
pub(super) fn detect_agents() -> Vec<DetectedAgent> {
    let dirs = search_dirs();
    AGENTS
        .iter()
        .filter_map(|spec| {
            find_program(spec.program, &dirs, EXTENSIONS).map(|path| DetectedAgent {
                key: spec.key,
                name: spec.name,
                path,
            })
        })
        .collect()
}

/// The local session an `agent:<key>` id opens, or `None` when the id is not
/// an agent launcher or the agent is no longer installed. The program is
/// resolved again on every click, so an agent installed or removed after
/// meatshell started is handled correctly.
pub(super) fn agent_session(id: &str) -> Option<Session> {
    let spec = agent_spec(id)?;
    let path = find_program(spec.program, &search_dirs(), EXTENSIONS)?;
    Some(agent_session_for(spec, &path))
}

/// Display name of an `agent:<key>` id, whether or not it is installed.
pub(super) fn agent_name(id: &str) -> Option<&'static str> {
    agent_spec(id).map(|spec| spec.name)
}

fn agent_spec(id: &str) -> Option<&'static AgentSpec> {
    let key = id.strip_prefix(AGENT_ID_PREFIX)?;
    AGENTS.iter().find(|spec| spec.key == key)
}

fn agent_session_for(spec: &AgentSpec, path: &Path) -> Session {
    #[cfg(windows)]
    let host = "powershell";
    #[cfg(not(windows))]
    let host = "shell";
    let mut session =
        builtin_local_session(&format!("{AGENT_ID_PREFIX}{}", spec.key), spec.name, host);
    session.group = AGENTS_GROUP.to_string();
    session.local_command = path.to_string_lossy().into_owned();
    session
}

/// Sessions for the built-in "agents" group in Quick connect, in `AGENTS`
/// order. Empty until the first detection pass has finished.
pub(super) fn detected_agent_sessions() -> Vec<Session> {
    DETECTED
        .get()
        .into_iter()
        .flatten()
        .filter_map(|agent| {
            AGENTS
                .iter()
                .find(|spec| spec.key == agent.key)
                .map(|spec| agent_session_for(spec, &agent.path))
        })
        .collect()
}

/// `PATH`, then the usual per-user install locations. The fallbacks cover an
/// agent installed after meatshell started (Windows processes keep the PATH
/// they were launched with) and macOS apps opened from Finder, which get a
/// minimal PATH.
fn search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from);
    if let Some(home) = &home {
        dirs.push(home.join(".local").join("bin"));
        dirs.push(home.join(".bun").join("bin"));
    }
    #[cfg(windows)]
    {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            dirs.push(PathBuf::from(appdata).join("npm"));
        }
    }
    #[cfg(not(windows))]
    {
        if let Some(home) = &home {
            dirs.push(home.join(".npm-global").join("bin"));
            dirs.push(home.join(".cargo").join("bin"));
        }
        dirs.push(PathBuf::from("/opt/homebrew/bin"));
        dirs.push(PathBuf::from("/usr/local/bin"));
    }
    dirs
}

/// First `dir/program<ext>` that is an executable file. Directories are
/// searched in order and every extension is tried per directory, matching how
/// shells resolve commands.
pub(super) fn find_program(
    program: &str,
    dirs: &[PathBuf],
    extensions: &[&str],
) -> Option<PathBuf> {
    dirs.iter()
        .filter(|dir| !dir.as_os_str().is_empty())
        .flat_map(|dir| {
            extensions.iter().map(move |ext| {
                let mut name = OsString::from(program);
                name.push(ext);
                dir.join(name)
            })
        })
        .find(|candidate| is_executable(candidate))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// Fill the tab bar's launch menu and the Quick connect "agents" group.
/// Detection touches the filesystem for every PATH entry, so the first window
/// runs it off the UI thread and never delays startup; later windows reuse
/// the result.
pub(super) fn wire_agent_launchers(window: &super::AppWindow) {
    if let Some(agents) = DETECTED.get() {
        show_agents(window, agents);
        return;
    }
    let weak = window.as_weak();
    let spawned = std::thread::Builder::new()
        .name("agent-detect".into())
        .spawn(move || {
            let agents = DETECTED.get_or_init(detect_agents);
            if agents.is_empty() {
                return;
            }
            tracing::info!(
                "agent launchers: {}",
                agents.iter().map(|a| a.name).collect::<Vec<_>>().join(", ")
            );
            let _ = weak.upgrade_in_event_loop(move |w| show_agents(&w, agents));
        });
    if let Err(err) = spawned {
        tracing::warn!("agent detection thread failed to start: {err}");
    }
}

fn show_agents(window: &super::AppWindow, agents: &[DetectedAgent]) {
    if agents.is_empty() {
        return;
    }
    let rows: Vec<super::LaunchItem> = agents
        .iter()
        .map(|agent| super::LaunchItem {
            id: format!("{AGENT_ID_PREFIX}{}", agent.key).into(),
            name: agent.name.into(),
        })
        .collect();
    window.set_agents(slint::ModelRc::new(slint::VecModel::from(rows)));
    // Quick connect rows read the detection cache when they are built, so
    // rebuild them (keeping the current search) to show the agents group.
    window.invoke_host_search_changed(window.get_host_search_query());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch_executable(path: &Path) {
        std::fs::write(path, b"").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    #[test]
    fn earlier_directory_wins_and_extensions_are_tried_in_order() {
        let root = std::env::temp_dir().join(format!("meatshell-agents-{}", uuid::Uuid::new_v4()));
        let first = root.join("first");
        let second = root.join("second");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        touch_executable(&first.join("codex.cmd"));
        touch_executable(&first.join("codex.exe"));
        touch_executable(&second.join("codex.exe"));

        let dirs = vec![PathBuf::new(), first.clone(), second.clone()];
        assert_eq!(
            find_program("codex", &dirs, &[".exe", ".cmd"]),
            Some(first.join("codex.exe"))
        );
        assert_eq!(
            find_program("codex", &dirs, &[".cmd", ".exe"]),
            Some(first.join("codex.cmd"))
        );
        assert_eq!(find_program("gemini", &dirs, &[".exe", ".cmd"]), None);
        // A directory with the program's name is not a match.
        std::fs::create_dir_all(second.join("qwen.exe")).unwrap();
        assert_eq!(find_program("qwen", &dirs, &[".exe"]), None);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn non_executable_files_are_skipped() {
        let dir = std::env::temp_dir().join(format!("meatshell-agents-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("claude"), b"").unwrap();
        assert_eq!(find_program("claude", &[dir.clone()], &[""]), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn agent_ids_map_to_local_sessions_running_the_program() {
        let spec = agent_spec("agent:claude").unwrap();
        let path = PathBuf::from("/opt/agents/claude");
        let session = agent_session_for(spec, &path);
        assert_eq!(session.id, "agent:claude");
        assert_eq!(session.name, "Claude Code");
        assert_eq!(session.kind, crate::config::SessionKind::Local);
        assert_eq!(session.group, AGENTS_GROUP);
        assert_eq!(session.local_command, path.to_string_lossy());

        assert_eq!(agent_name("agent:cursor"), Some("Cursor CLI"));
        assert!(agent_spec("agent:unknown").is_none());
        assert!(agent_spec("system:powershell").is_none());
    }

    #[test]
    fn agent_keys_are_unique() {
        let mut keys: Vec<_> = AGENTS.iter().map(|spec| spec.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), AGENTS.len());
    }
}
