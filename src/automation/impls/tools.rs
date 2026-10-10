use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};

use crate::config::{ConfigStore, Session};

use super::structs::Frontend;

const DEFAULT_TIMEOUT_SECONDS: u64 = 30;
const MAX_TIMEOUT_SECONDS: u64 = 300;
const DEFAULT_MAX_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_OUTPUT_BYTES: usize = 4 * 1024 * 1024;

pub(crate) async fn call(name: &str, arguments: &Value, frontend: Frontend) -> Result<Value> {
    crate::ssh::with_automation_cancellation(call_inner(name, arguments, frontend)).await
}

async fn call_inner(name: &str, arguments: &Value, frontend: Frontend) -> Result<Value> {
    match name {
        "list_sessions" => list_sessions(arguments, frontend),
        "get_session" => get_session(arguments, frontend),
        "import_sessions" => import_sessions(arguments, frontend),
        "run_command" => run_command(arguments, frontend).await,
        "list_remote_files" => list_remote_files(arguments, frontend).await,
        "read_remote_text_file" => read_remote_text_file(arguments, frontend).await,
        "upload_file" => upload_file(arguments, frontend).await,
        "download_file" => download_file(arguments, frontend).await,
        _ => Err(anyhow!("unknown tool: {name}")),
    }
}

fn import_sessions(arguments: &Value, frontend: Frontend) -> Result<Value> {
    let path = required_string(arguments, "local_path")?;
    if path.trim().is_empty() {
        return Err(anyhow!("local_path must not be empty"));
    }
    // Preview is the safe default for protocol callers. The CLI passes its
    // explicit --dry-run choice; a server process must opt in before applying.
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    if !dry_run && !frontend.allows_config_import() {
        return Err(anyhow!(
            "configuration import is disabled; restart MCP with --allow-config-import to apply imports"
        ));
    }
    let mut store = load_store(frontend)?;
    enforce_transfer_permissions(&store, frontend)?;
    let result = store.import_from_preview(std::path::Path::new(path), dry_run)?;
    // Never return source JSON, credential material, or private file paths.
    Ok(json!({ "added": result.added, "skipped": result.skipped, "dry_run": dry_run }))
}

async fn upload_file(arguments: &Value, frontend: Frontend) -> Result<Value> {
    let store = load_store(frontend)?;
    enforce_transfer_permissions(&store, frontend)?;
    drop(store);
    let (session, jump, timeout) = sftp_context(arguments, frontend)?;
    let local_path = std::path::PathBuf::from(required_string(arguments, "local_path")?);
    if !local_path.is_file() {
        return Err(anyhow!(
            "local upload source is not a regular file: {}",
            local_path.display()
        ));
    }
    if frontend == Frontend::Mcp {
        enforce_upload_sandbox(&local_path)?;
    }
    let remote_directory = required_string(arguments, "remote_directory")?;
    // Report where the file actually landed, the way download_file reports
    // local_path: the caller cannot derive it without knowing how the worker
    // joins the directory and the name.
    let file_name = local_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            anyhow!(
                "local file name is not valid UTF-8: {}",
                local_path.display()
            )
        })?;
    let remote_path = crate::sftp::upload_target_path(remote_directory, file_name);
    let mut result = super::sftp::transfer(
        session,
        jump,
        crate::sftp::SftpCommand::Upload {
            local: local_path,
            remote_dir: remote_directory.to_string(),
            cleanup_after: None,
        },
        true,
        timeout,
    )
    .await?;
    if let Some(object) = result.as_object_mut() {
        object.insert("remote_path".to_string(), json!(remote_path));
    }
    Ok(result)
}

async fn download_file(arguments: &Value, frontend: Frontend) -> Result<Value> {
    let store = load_store(frontend)?;
    enforce_transfer_permissions(&store, frontend)?;
    drop(store);
    let (session, jump, timeout) = sftp_context(arguments, frontend)?;
    let remote_path = required_string(arguments, "remote_path")?;
    let local_directory = std::path::PathBuf::from(required_string(arguments, "local_directory")?);
    if !local_directory.is_dir() {
        return Err(anyhow!(
            "local download destination is not an existing directory: {}",
            local_directory.display()
        ));
    }
    // The destination file name is the last segment of the remote path, split on
    // both separators; it must be a plain, non-traversing name (#53).
    let file_name = remote_path
        .trim_end_matches(['/', '\\'])
        .rsplit(|c| c == '/' || c == '\\')
        .next()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| anyhow!("remote_path must identify a file"))?;
    validate_download_file_name(file_name)?;

    // Resolve the real destination the SFTP worker writes (it sanitizes the
    // basename), normalize the directory, and assert the target stays inside it.
    let canonical_dir = local_directory
        .canonicalize()
        .context("canonicalize local download directory")?;
    let target = crate::sftp::download_target_path(&remote_path, &canonical_dir.to_string_lossy());
    if !target.starts_with(&canonical_dir) {
        return Err(anyhow!(
            "download destination escapes local_directory: {}",
            target.display()
        ));
    }
    // Refuse to overwrite, matching the tool's documented promise (#53).
    if target.exists() {
        return Err(anyhow!(
            "download destination already exists: {}",
            target.display()
        ));
    }

    let mut result = super::sftp::transfer(
        session,
        jump,
        crate::sftp::SftpCommand::Download {
            remote: remote_path.to_string(),
            local_dir: canonical_dir.to_string_lossy().into_owned(),
            // Never overwrite an existing file, even in the race between the
            // exists() check above and the actual write (#53).
            conflict: crate::sftp::DownloadConflict::KeepBoth,
        },
        false,
        timeout,
    )
    .await?;
    if let Some(object) = result.as_object_mut() {
        object.insert("local_path".to_string(), json!(target.to_string_lossy()));
    }
    Ok(result)
}

fn enforce_transfer_permissions(store: &ConfigStore, frontend: Frontend) -> Result<()> {
    if frontend.is_mcp() && !store.mcp_allow_file_transfers() {
        return Err(anyhow!(
            "file transfers are disabled in Settings > Interface > MCP"
        ));
    }
    Ok(())
}

/// #54: MCP upload sources must come from inside a controlled directory — the
/// process working directory — and must not traverse out of it. The MCP server
/// is an external stdio process whose caller is not authenticated, so an
/// unrestricted `local_path` would let a prompt-injected agent exfiltrate
/// arbitrary local files (e.g. `~/.ssh/id_rsa`) to a saved remote host.
fn enforce_upload_sandbox(local_path: &std::path::Path) -> Result<()> {
    use std::path::Component;

    if local_path.components().any(|component| component == Component::ParentDir) {
        return Err(anyhow!(
            "upload source must not contain '..': {}",
            local_path.display()
        ));
    }
    let cwd = std::env::current_dir()
        .context("resolve the MCP upload sandbox (process working directory)")?;
    let canonical_cwd = cwd.canonicalize().unwrap_or(cwd);
    let canonical_source = local_path
        .canonicalize()
        .context("canonicalize upload source")?;
    if !canonical_source.starts_with(&canonical_cwd) {
        tracing::warn!(
            "mcp upload_file rejected: source {} is outside the allowed transfer directory {}",
            canonical_source.display(),
            canonical_cwd.display()
        );
        return Err(anyhow!(
            "upload source is outside the allowed transfer directory (process working directory): {}",
            local_path.display()
        ));
    }
    Ok(())
}

/// Reject a derived download file name that could escape the destination
/// directory: blank names, names containing `..`, or anything that is not a
/// single plain path segment (#53).
fn validate_download_file_name(file_name: &str) -> Result<()> {
    if file_name.trim().is_empty() {
        return Err(anyhow!("remote file name must not be blank"));
    }
    if file_name.contains("..") {
        return Err(anyhow!(
            "remote file name must not contain '..': {file_name:?}"
        ));
    }
    let as_path = std::path::Path::new(file_name);
    if as_path.is_absolute() || as_path.components().count() != 1 || as_path.file_name().is_none() {
        return Err(anyhow!(
            "remote file name must be a plain file name, not a path: {file_name:?}"
        ));
    }
    Ok(())
}

/// #55: second gate for MCP command execution. Besides the `mcp_allow_commands`
/// config switch, an operator can restrict which sessions and command prefixes
/// the MCP server may run via environment variables:
///   - `MEATSHELL_MCP_ALLOWED_SESSIONS`: comma-separated session ids or names.
///   - `MEATSHELL_MCP_COMMAND_PREFIXES`: comma-separated command prefixes.
/// A set variable is enforced (fail closed); an unset variable leaves the
/// existing behaviour unchanged so authorized use keeps working. Every command
/// execution is also audited with session + caller by the MCP server.
fn enforce_mcp_command_gate(session: &Session, command: &str) -> Result<()> {
    if let Some(allowlist) = nonempty_env("MEATSHELL_MCP_ALLOWED_SESSIONS") {
        let allowed: Vec<&str> = allowlist
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .collect();
        let permitted = allowed
            .iter()
            .any(|entry| *entry == session.id.as_str() || *entry == session.name.as_str());
        if !permitted {
            return Err(anyhow!(
                "session is not in the MCP command allowlist (MEATSHELL_MCP_ALLOWED_SESSIONS): {}",
                session.name
            ));
        }
    }
    if let Some(prefixes) = nonempty_env("MEATSHELL_MCP_COMMAND_PREFIXES") {
        let allowed: Vec<&str> = prefixes
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .collect();
        let trimmed = command.trim_start();
        let permitted = allowed.iter().any(|prefix| trimmed.starts_with(prefix));
        if !permitted {
            return Err(anyhow!(
                "command does not match any allowed prefix (MEATSHELL_MCP_COMMAND_PREFIXES)"
            ));
        }
    }
    Ok(())
}

/// Read an env var and return `Some` only when it is present and non-empty.
fn nonempty_env(key: &str) -> Option<String> {
    match std::env::var(key) {
        Ok(value) if !value.trim().is_empty() => Some(value),
        _ => None,
    }
}

async fn list_remote_files(arguments: &Value, frontend: Frontend) -> Result<Value> {
    let (session, jump, timeout) = sftp_context(arguments, frontend)?;
    let path = optional_string(arguments, "path")?
        .unwrap_or(".")
        .to_string();
    super::sftp::list(session, jump, path, timeout).await
}

async fn read_remote_text_file(arguments: &Value, frontend: Frontend) -> Result<Value> {
    let (session, jump, timeout) = sftp_context(arguments, frontend)?;
    let path = required_string(arguments, "path")?;
    if path.trim().is_empty() {
        return Err(anyhow!("path must not be empty"));
    }
    super::sftp::read_text(session, jump, path.to_string(), timeout).await
}

fn sftp_context(
    arguments: &Value,
    frontend: Frontend,
) -> Result<(Session, Vec<Session>, Duration)> {
    let store = load_store(frontend)?;
    if frontend.is_mcp() && !store.mcp_use_saved_credentials() {
        return Err(anyhow!(
            "using saved credentials is disabled in Settings > Interface > MCP"
        ));
    }
    let reference = required_string(arguments, "session_id")?;
    let session = visible_session(&store, reference, frontend)?.clone();
    if session.kind.as_str() != "ssh" {
        return Err(anyhow!("SFTP tools only support SSH sessions"));
    }
    let jump = visible_jump_chain(&store, &session, frontend)?;
    let timeout = optional_u64(arguments, "timeout_seconds")?
        .unwrap_or(DEFAULT_TIMEOUT_SECONDS)
        .clamp(1, MAX_TIMEOUT_SECONDS);
    Ok((session, jump, Duration::from_secs(timeout)))
}

fn load_store(frontend: Frontend) -> Result<ConfigStore> {
    let store = ConfigStore::load().context("load MeatShell configuration")?;
    if frontend.is_mcp() && !store.mcp_enabled() {
        return Err(anyhow!("MCP is disabled in Settings > Interface > MCP"));
    }
    Ok(store)
}

/// Resolve a caller-supplied session reference: an exact id first, then an
/// exact name, then a unique case-insensitive name match. Session names are
/// not unique, so an ambiguous name is an error that lists the candidate ids
/// so the caller can disambiguate.
fn resolve_session<'a>(store: &'a ConfigStore, reference: &str) -> Result<&'a Session> {
    let reference = reference.trim();
    if reference.is_empty() {
        return Err(anyhow!("session id or name must not be empty"));
    }
    if let Some(session) = store.get(reference) {
        return Ok(session);
    }
    let exact: Vec<&Session> = store
        .sessions()
        .iter()
        .filter(|session| session.name == reference)
        .collect();
    match exact.len() {
        1 => return Ok(exact[0]),
        n if n > 1 => {
            let ids = exact
                .iter()
                .map(|session| session.id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(anyhow!(
                "session name is ambiguous ({n} matches), pass the session_id: {ids}"
            ));
        }
        _ => {}
    }
    let case_insensitive: Vec<&Session> = store
        .sessions()
        .iter()
        .filter(|session| session.name.eq_ignore_ascii_case(reference))
        .collect();
    match case_insensitive.len() {
        1 => Ok(case_insensitive[0]),
        0 => Err(anyhow!("session not found: {reference}")),
        _ => Err(anyhow!(
            "session name is ambiguous ({} matches), pass the session_id",
            case_insensitive.len()
        )),
    }
}

/// Per-session MCP scope (#432). The CLI is an explicit local user action and
/// always sees every session.
fn exposed_to(session: &Session, frontend: Frontend) -> bool {
    !frontend.is_mcp() || session.mcp_access
}

/// Look up a session the caller may use, by id or display name (see
/// [`resolve_session`]). Sessions hidden from MCP report the same error as
/// missing ones so their existence is not disclosed.
fn visible_session<'a>(
    store: &'a ConfigStore,
    reference: &str,
    frontend: Frontend,
) -> Result<&'a Session> {
    let session = resolve_session(store, reference)?;
    if exposed_to(session, frontend) {
        Ok(session)
    } else {
        Err(anyhow!("session not found: {reference}"))
    }
}

/// Resolve the jump route, refusing it when any hop is hidden from MCP:
/// routing through a hop authenticates with that hop's saved credentials.
fn visible_jump_chain(
    store: &ConfigStore,
    session: &Session,
    frontend: Frontend,
) -> Result<Vec<Session>> {
    let jump = store.resolve_jump_chain(session)?;
    ensure_hops_exposed(&jump, frontend)?;
    Ok(jump)
}

fn ensure_hops_exposed(jump: &[Session], frontend: Frontend) -> Result<()> {
    if jump.iter().any(|hop| !exposed_to(hop, frontend)) {
        return Err(anyhow!(
            "this session routes through a jump host that does not allow MCP access"
        ));
    }
    Ok(())
}

fn list_sessions(arguments: &Value, frontend: Frontend) -> Result<Value> {
    let store = load_store(frontend)?;
    let group = optional_string(arguments, "group")?;
    let sessions: Vec<Value> = store
        .sessions()
        .iter()
        .filter(|session| exposed_to(session, frontend))
        .filter(|session| group.map_or(true, |group| session.group == group))
        .map(safe_session)
        .collect();
    Ok(json!({ "sessions": sessions }))
}

fn get_session(arguments: &Value, frontend: Frontend) -> Result<Value> {
    let store = load_store(frontend)?;
    let reference = required_string(arguments, "session_id")?;
    Ok(safe_session(visible_session(&store, reference, frontend)?))
}

async fn run_command(arguments: &Value, frontend: Frontend) -> Result<Value> {
    let store = load_store(frontend)?;
    if frontend.is_mcp() && !store.mcp_use_saved_credentials() {
        return Err(anyhow!(
            "using saved credentials is disabled in Settings > Interface > MCP"
        ));
    }
    if frontend.is_mcp() && !store.mcp_allow_commands() {
        return Err(anyhow!(
            "arbitrary command execution is disabled in Settings > Interface > MCP"
        ));
    }

    let reference = required_string(arguments, "session_id")?;
    let command = required_string(arguments, "command")?;
    if command.trim().is_empty() {
        return Err(anyhow!("command must not be empty"));
    }
    let timeout_seconds = optional_u64(arguments, "timeout_seconds")?
        .unwrap_or(DEFAULT_TIMEOUT_SECONDS)
        .clamp(1, MAX_TIMEOUT_SECONDS);
    let max_output_bytes = optional_u64(arguments, "max_output_bytes")?
        .unwrap_or(DEFAULT_MAX_OUTPUT_BYTES as u64)
        .clamp(1024, MAX_OUTPUT_BYTES as u64) as usize;

    let session = visible_session(&store, reference, frontend)?.clone();
    if session.kind.as_str() != "ssh" {
        return Err(anyhow!("run_command only supports SSH sessions"));
    }
    if frontend == Frontend::Mcp {
        enforce_mcp_command_gate(&session, command)?;
    }
    let jump = visible_jump_chain(&store, &session, frontend)?;

    let result = crate::ssh::execute_command(
        session,
        jump,
        command,
        Duration::from_secs(timeout_seconds),
        max_output_bytes,
    )
    .await?;
    serde_json::to_value(result).context("serialize command result")
}

fn safe_session(session: &Session) -> Value {
    json!({
        "id": session.id,
        "name": session.name,
        "kind": session.kind.as_str(),
        "host": session.host,
        "port": session.port,
        "user": session.user,
        "auth": session.auth.as_str(),
        "group": session.group,
        "has_saved_password": !session.password.is_empty(),
        "has_private_key": !session.private_key_path.trim().is_empty()
            || !session.private_key_inline.is_empty(),
        "jump_session_id": session.jump_session_id,
        "jump_session_ids": session.jump_session_ids,
        "has_proxy": !session.proxy.trim().is_empty(),
    })
}

fn required_string<'a>(arguments: &'a Value, key: &str) -> Result<&'a str> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing or invalid string argument: {key}"))
}

fn optional_string<'a>(arguments: &'a Value, key: &str) -> Result<Option<&'a str>> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .map(Some)
            .ok_or_else(|| anyhow!("invalid string argument: {key}")),
    }
}

fn optional_bool(arguments: &Value, key: &str) -> Result<Option<bool>> {
    match arguments.get(key) {
        None => Ok(None),
        Some(value) => value
            .as_bool()
            .map(Some)
            .ok_or_else(|| anyhow!("invalid boolean argument: {key}")),
    }
}

fn optional_u64(arguments: &Value, key: &str) -> Result<Option<u64>> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| anyhow!("invalid positive integer argument: {key}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_permissions_are_explicit() {
        assert!(Frontend::Cli.allows_config_import());
        assert!(!Frontend::Mcp {
            allow_config_import: false
        }
        .allows_config_import());
        assert!(Frontend::Mcp {
            allow_config_import: true
        }
        .allows_config_import());
        assert_eq!(optional_bool(&json!({}), "dry_run").unwrap(), None);
        assert_eq!(
            optional_bool(&json!({"dry_run": false}), "dry_run").unwrap(),
            Some(false)
        );
        assert!(optional_bool(&json!({"dry_run": "false"}), "dry_run").is_err());
        assert!(optional_bool(&json!({"dry_run": null}), "dry_run").is_err());
        let err = import_sessions(
            &json!({"local_path": "unused", "dry_run": false}),
            Frontend::Mcp {
                allow_config_import: false,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("--allow-config-import"));
    }

    #[test]
    fn mcp_scope_hides_opted_out_sessions_from_mcp_only() {
        let mcp = Frontend::Mcp {
            allow_config_import: false,
        };
        let mut hidden = Session::new_empty();
        hidden.mcp_access = false;
        let shown = Session::new_empty();
        assert!(!exposed_to(&hidden, mcp));
        assert!(exposed_to(&shown, mcp));
        assert!(exposed_to(&hidden, Frontend::Cli));

        assert!(ensure_hops_exposed(&[shown.clone()], mcp).is_ok());
        let err = ensure_hops_exposed(&[shown.clone(), hidden.clone()], mcp).unwrap_err();
        assert!(err.to_string().contains("jump host"));
        assert!(ensure_hops_exposed(&[hidden], Frontend::Cli).is_ok());
    }

    #[test]
    fn mcp_access_defaults_on_for_new_and_legacy_sessions() {
        let session = Session::new_empty();
        assert!(session.mcp_access);
        let mut legacy = serde_json::to_value(&session).unwrap();
        legacy.as_object_mut().unwrap().remove("mcp_access");
        let restored: Session = serde_json::from_value(legacy).unwrap();
        assert!(restored.mcp_access);
        let mut opted_out = session;
        opted_out.mcp_access = false;
        let json = serde_json::to_value(&opted_out).unwrap();
        assert!(!serde_json::from_value::<Session>(json).unwrap().mcp_access);
    }

    #[test]
    fn numeric_arguments_are_strict() {
        assert_eq!(optional_u64(&json!({}), "n").unwrap(), None);
        assert_eq!(optional_u64(&json!({ "n": 12 }), "n").unwrap(), Some(12));
        assert!(optional_u64(&json!({ "n": -1 }), "n").is_err());
        assert!(optional_u64(&json!({ "n": "12" }), "n").is_err());
    }

    fn store_with(sessions: Vec<Session>) -> ConfigStore {
        ConfigStore {
            path: std::env::temp_dir().join("ms-automation-sessions.json"),
            backup_dir: None,
            cache: crate::config::ConfigFile {
                sessions,
                ..Default::default()
            },
            key: [7u8; 32],
        }
    }

    fn session(id: &str, name: &str, host: &str) -> Session {
        Session {
            id: id.into(),
            name: name.into(),
            host: host.into(),
            ..Session::new_empty()
        }
    }

    #[test]
    fn resolves_sessions_by_id_and_name() {
        let store = store_with(vec![
            session("id-a", "Alpha", "10.0.0.1"),
            session("id-b", "Beta", "10.0.0.2"),
        ]);

        assert_eq!(resolve_session(&store, "id-a").unwrap().name, "Alpha");
        assert_eq!(resolve_session(&store, "Alpha").unwrap().id, "id-a");
        assert_eq!(resolve_session(&store, "beta").unwrap().id, "id-b");
        assert!(resolve_session(&store, "Gamma").is_err());
        assert!(resolve_session(&store, "").is_err());
    }

    #[test]
    fn rejects_ambiguous_session_names() {
        let store = store_with(vec![
            session("id-a", "prod", "10.0.0.1"),
            session("id-b", "prod", "10.0.0.2"),
        ]);

        let message = resolve_session(&store, "prod").unwrap_err().to_string();
        assert!(message.contains("ambiguous"));
        assert!(message.contains("id-a"));
        assert!(message.contains("id-b"));
    }
}
