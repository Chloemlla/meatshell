use anyhow::Result;
use serde_json::{json, Value};

pub(super) fn definitions() -> Value {
    json!([
        {
            "name": "list_sessions",
            "title": "List saved sessions",
            "description": "List the SSH sessions saved in MeatShell, without exposing passwords, private keys, or other secrets. Start here: every other tool needs a session, and a session's display name works anywhere session_id is accepted.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "group": { "type": "string", "description": "Optional exact session group filter." }
                },
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "idempotentHint": true, "openWorldHint": false }
        },
        {
            "name": "get_session",
            "title": "Inspect one session",
            "description": "Get one saved session's non-secret connection metadata: host, port, user, auth method, group, jump session, and whether a password or key is stored. Use it to confirm which machine a name points at before running anything.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "session_id": { "type": "string", "description": "Session id or display name from list_sessions." }
                },
                "required": ["session_id"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "idempotentHint": true, "openWorldHint": false }
        },
        {
            "name": "run_command",
            "title": "Run a remote command",
            "description": "Run one non-interactive command over SSH and return its output. Each call opens its own connection and closes it again, so nothing carries over between calls: no working directory, no shell variables, no background jobs, and no ~/.bashrc. Chain the steps of one job into a single command instead (`cd /srv && ./deploy.sh; systemctl status app --no-pager`), and use absolute paths for anything outside the default PATH. stdin is closed, so anything that waits for input (interactive sudo, a confirmation prompt, a pager) only burns the timeout — pass `sudo -n`, `-y`, `--no-pager`. Work that must outlive the call has to be detached (`nohup … >/tmp/job.log 2>&1 &`, `setsid`, `systemd-run`) and polled by a later call. Returns stdout, stderr, exit_code, timed_out, truncated, and exit_signal when a signal killed the command. A non-zero exit_code is a normal result, not an error; exit_code is null only when the command was signalled or the connection went away (rebooting the host, restarting sshd). Requires the saved-credentials and arbitrary-command permissions in Settings > Interface > MCP.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "session_id": { "type": "string", "description": "Session id or display name from list_sessions." },
                    "command": { "type": "string", "minLength": 1, "description": "Run non-interactively by the remote user's shell. Never put a password, token, or key in here: the call is written to MeatShell's MCP activity log." },
                    "timeout_seconds": { "type": "integer", "minimum": 1, "maximum": 300, "default": 30, "description": "Hard ceiling of 300. On timeout the result carries timed_out with no output, and dropping the connection usually kills the remote command — detach long jobs instead of raising this." },
                    "max_output_bytes": { "type": "integer", "minimum": 1024, "maximum": 4194304, "default": 1048576, "description": "Cap per stream; the result sets truncated when output was cut. Prefer filtering on the server (grep, tail, --no-pager) over raising this." }
                },
                "required": ["session_id", "command"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": true, "idempotentHint": false, "openWorldHint": true }
        },
        {
            "name": "list_remote_files",
            "title": "List a remote directory",
            "description": "List one remote directory over SFTP, non-recursively. Returns each entry's name, absolute path, is_directory, size, modified, and mode (4-digit octal). Prefer this over shelling out to ls; for a recursive walk use run_command with find. Requires the saved-credentials permission.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "session_id": { "type": "string", "description": "Session id or display name from list_sessions." },
                    "path": { "type": "string", "default": ".", "description": "Absolute remote directory. \".\" lists the login directory." },
                    "timeout_seconds": { "type": "integer", "minimum": 1, "maximum": 300, "default": 30 }
                },
                "required": ["session_id"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "idempotentHint": true, "openWorldHint": true }
        },
        {
            "name": "read_remote_text_file",
            "title": "Read a remote text file",
            "description": "Read a remote UTF-8 text file over SFTP and return its full content. Bounded on purpose: at most 512 KiB, 20000 lines, and 64 KiB on any single line; binary bytes and invalid UTF-8 are rejected rather than mangled. For a bigger or binary file use download_file, or read a slice with run_command (sed -n, tail, head). Requires the saved-credentials permission.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "session_id": { "type": "string", "description": "Session id or display name from list_sessions." },
                    "path": { "type": "string", "minLength": 1, "description": "Absolute path of the remote file." },
                    "timeout_seconds": { "type": "integer", "minimum": 1, "maximum": 300, "default": 30 }
                },
                "required": ["session_id", "path"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "idempotentHint": true, "openWorldHint": true }
        },
        {
            "name": "upload_file",
            "title": "Upload a file",
            "description": "Upload one local file into an existing remote directory over SFTP, keeping its file name. The write is atomic (staged under a temp name, then renamed into place) and it replaces a file that is already there, keeping that file's permissions; a brand-new file lands on the server's default mode, so tighten it with run_command chmod when the content is sensitive. Returns the bytes transferred and the resulting remote_path. Requires the saved-credentials and file-transfer permissions.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "session_id": { "type": "string", "description": "Session id or display name from list_sessions." },
                    "local_path": { "type": "string", "minLength": 1, "description": "A regular file inside the directory the MCP server process was started in, with no \"..\" segments — anything else is refused. This is also the way to move a secret onto a host without putting it in a command line." },
                    "remote_directory": { "type": "string", "minLength": 1, "description": "Existing remote directory. It is not created for you." },
                    "timeout_seconds": { "type": "integer", "minimum": 1, "maximum": 300, "default": 120 }
                },
                "required": ["session_id", "local_path", "remote_directory"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": true, "idempotentHint": true, "openWorldHint": true }
        },
        {
            "name": "download_file",
            "title": "Download a file",
            "description": "Download one remote file into an existing local directory over SFTP. It never overwrites: if a file of that name is already there the call fails, so move the old one aside first. Returns the bytes transferred and the resulting local_path. Requires the saved-credentials and file-transfer permissions.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "session_id": { "type": "string", "description": "Session id or display name from list_sessions." },
                    "remote_path": { "type": "string", "minLength": 1, "description": "Absolute path of the remote file." },
                    "local_directory": { "type": "string", "minLength": 1, "description": "Existing local directory. It is not created for you." },
                    "timeout_seconds": { "type": "integer", "minimum": 1, "maximum": 300, "default": 120 }
                },
                "required": ["session_id", "remote_path", "local_directory"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": true }
        }
    ])
}

pub(super) async fn call_mcp(name: &str, arguments: &Value) -> Result<Value> {
    crate::automation::call(name, arguments, crate::automation::Frontend::Mcp).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definitions_do_not_expose_secret_arguments() {
        let text = definitions().to_string();
        assert!(!text.contains("\"password\":"));
        assert!(!text.contains("\"private_key_inline\":"));
        assert!(text.contains("run_command"));
    }
}
