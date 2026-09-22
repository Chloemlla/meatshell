# meatshell

[简体中文](./README.md) | **English**

A lightweight, low-memory SSH / terminal client inspired by FinalShell, but
written entirely in **Rust + [Slint](https://slint.dev)**. The goal is to keep
FinalShell's core experience (resource-monitor sidebar, session management,
tabbed terminals) while cutting memory use from the 400 MB+ of a JVM app down to
the tens-of-MB range of a native binary.

## ⚡ Fork Enhancements

This branch (`Chloemlla/meatshell`) continues from upstream `yituorou/meatshell` and
builds its own releases. The table lists work that exists only here; see
[CHANGELOG.md](CHANGELOG.md) for the per-change detail.

| Feature | What it does |
| ------- | ------------ |
| **Live MCP activity audit** | Settings → Interface → MCP polls `mcp_activity.jsonl` once a second and shows what an AI client has done through `meatshell mcp serve` (time, caller, tool, command / path, status, duration), with Refresh / Clear. Only whitelisted arguments are written, inline passwords and tokens in commands are redacted first, and the file is trimmed to the newest 1000 records past 2 MiB. |
| **Expandable activity rows** | A collapsed row folds the command's newlines into a one-line summary; clicking expands it to the full command text, growing the row to fit, with a chevron marking the state, so multi-line commands no longer overlap each other. |
| **Operate sessions by name** | The `session_id` argument of the CLI and MCP tools also accepts a session's display name (exact id first, then exact name, then a unique case-insensitive name); an ambiguous name errors and lists the candidate ids, so an AI need not look up an id first. |
| **Complete MCP tool contract** | All seven tools gained a `title`, an operational description, and `readOnlyHint` / `destructiveHint` / `idempotentHint` / `openWorldHint` annotations, with per-argument limits spelled out: the 300-second timeout ceiling, the output cap, the 512 KiB / 20000 lines / 64 KiB-per-line bounds on text reads, and the upload sandbox and replace semantics. |
| **Remote commands no longer report false success** | When the server refused the exec request, or the channel closed before an exit status arrived, `run_command` now errors instead of returning an empty "success"; a command killed by a signal reports a new `exit_signal` field, which the CLI prints too. |
| **Optional MCP allowlists** | `MEATSHELL_MCP_ALLOWED_SESSIONS` and `MEATSHELL_MCP_COMMAND_PREFIXES` restrict which sessions and command prefixes MCP may use to an explicit list. |
| **Replacing an existing remote file** | SFTP `rename` will not overwrite an existing target, so uploading a file that was already there, saving from the built-in editor, uploading a folder, and "copy to" all failed with `rename remote …: Failure`. The original is now moved aside first, the replacement keeps its permission bits, the displaced copy is removed on success, and the original name is restored if any step fails. The GUI, the CLI `upload` command, and the MCP `upload_file` tool share one implementation. |
| **Tighter local writes and file opening** | SFTP downloads land through a unique temp file plus rename; opening externally blocks dangerous executables; `read_dir` rejects slash-bearing names; closing SFTP cancels tracked transfers and aborts the rest. |
| **Tighter SSH edges** | `https://` outbound proxies are refused; CONNECT credentials and tokens are held in `Zeroizing` buffers; `known_hosts` writes are serialized under a lock onto an fsync'd unique temp file and refuse symlink targets; Argon2 cost caps are tightened; suppress-echo is bounded by time and bytes; passwords and keyboard-interactive responses are `Zeroizing`. |
| **Atomic credential writes** | `secret.key` is written atomically (pid+uuid temp file, 0600, rename) and the legacy plaintext is removed after a successful migration; `is_encrypted()` judges by a successful decryption instead of prefix matching; a damaged key is backed up as `secret.key.broken` before regeneration; legacy DES decryption keeps key material and plaintext in `Zeroizing` buffers. |
| **Bounded terminal memory** | ZMODEM receive caps subpackets at 1 MiB, session bytes at 4 GiB and files at 64, deleting a partial file unless ZEOF arrived; Telnet subnegotiation force-returns to Data past 4096 bytes / 30 s; serial writes move to a separate thread without `tcdrain`; the local PTY reader joins and ends its child process on exit. |
| **No blocking work on the UI thread** | WebDAV upload / download, system-metric sampling, and MCP activity polling run off the UI thread (`spawn_blocking` with results posted back to the event loop and re-entry guards); high-frequency config writes are debounced; clipboard copies use a single background worker; a window's background tasks are aborted when it closes; poisoned locks degrade instead of panicking; the export notice now says passwords are obfuscated, not encrypted. |
| **58 architecture-audit findings resolved** | Eight parallel read-only passes over about 32k lines of `src/` produced 58 findings (12 critical / 22 high / 21 medium / 3 low), each fixed with its status and commit hash recorded: 53 fixed and CI-verified, 3 explicitly skipped, 2 ruled false positives. See [docs/architecture-audit-2026-08-30.md](docs/architecture-audit-2026-08-30.md). |
| **A downloadable build for every push to `main`** | A successful build on `main` is auto-published as `v<version>-ci-<short-sha>` and marked Latest as a full release rather than a prerelease, so every commit has artifacts you can download. |
| **Update checks point at this repository** | The in-app update check and the Footer / About links point at `Chloemlla/meatshell`; the Flatpak app ID is `io.github.chloemlla.meatshell`; `Cargo.toml`'s `repository` and the AUR `PKGBUILD` url / Maintainer were updated too, so a build from this repository is not sent to upstream. |
| **Release caches that are actually reused** | Three defects where a cache looked configured but never hit are fixed: Flatpak no longer mints a fresh key from `github.sha`; the AppImage tooling is restored from a dedicated cache and only downloaded on a miss; the cache save is its own step guarded by `hashFiles()`, so a tolerated download failure cannot fail an otherwise green build. |
| **Build and cross-platform fixes** | Fixes code the upstream merge brought in that fails to compile on every target (`Layout` never derived `Clone`), along with cross-platform compile errors and unused imports, and removes dead code left behind by an upstream feature that was decommissioned. |

## Screenshots

<p align="center">
  <img src="docs/screenshots/01-welcome-en.png" alt="Welcome / session management" width="800"><br>
  <em>Welcome page: session management + local resource monitor sidebar</em>
</p>

<p align="center">
  <img src="docs/screenshots/02-terminal-htop.png" alt="Terminal + SFTP" width="800"><br>
  <em>Tabbed terminal (full-screen btop) + SFTP file browser + remote resource monitoring</em>
</p>

## Download & install

One GitHub Actions workflow produces everything on the
[Releases](https://github.com/Chloemlla/meatshell/releases) page, published along two paths:

- **Push to `main`**: a successful build is published as `v<version>-ci-<short-sha>`
  (for example `v0.7.3-ci-8d4f9dbf`) and marked as the repository's Latest, with asset
  names normalised to `meatshell-<version>-*`, so every commit on main has a build you
  can download.
- **Push a `v*` tag**: an official release, with the same assets attached to that tag,
  the version taken from `Cargo.toml`.

Each build covers Windows / Linux / macOS on both x86_64 and aarch64:

| Platform      | Files                                                                                                                                              |
| ------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| Windows       | `meatshell-*-windows-x86_64.zip`, `meatshell-*-windows-x86_64.msi`                                                                                 |
| Linux x86_64  | `meatshell-*-linux-x86_64.tar.gz`, `meatshell-*-linux-x86_64-glibc228.tar.gz`, `meatshell-*-linux-x86_64.AppImage`, `meatshell-*-linux-x86_64.flatpak` |
| Linux aarch64 | `meatshell-*-linux-aarch64.tar.gz`, `meatshell-*-linux-aarch64-glibc228.tar.gz`                                                                     |
| Debian/Ubuntu | `meatshell_*-1_amd64.deb`, `meatshell_*-1_arm64.deb`                                                                                               |
| macOS         | `meatshell-*-macos-aarch64.zip` (Apple silicon), `meatshell-*-macos-x86_64.zip` (Intel)                                                             |

### Windows

Download `meatshell-*-windows-x86_64.zip`, unzip, and run `meatshell.exe`; the
`meatshell-*-windows-x86_64.msi` installer works too.

### Linux

```bash
tar -xzf meatshell-*-linux-x86_64.tar.gz
cd meatshell-*-linux-x86_64
./meatshell                                  # run it directly
# Optional: system-wide install of the binary, icon, and launcher (requires sudo)
chmod +x install-linux.sh && ./install-linux.sh
```

The installer places the binary at `/usr/local/bin/meatshell`, the launcher at
`/usr/local/share/applications/meatshell.desktop`, and the icon at
`/usr/local/share/icons/hicolor/512x512/apps/meatshell.png`. It also removes a
stale same-named user launcher left by older tarball installers.

> The plain `linux-x86_64.tar.gz` is built on Ubuntu 22.04 and requires glibc ≥ 2.35
> (Ubuntu 22.04+ / Debian 12+); older distributions (CentOS 8, Debian 10, Ubuntu 18.04,
> …) should take the `-glibc228` variant instead, which is built inside a Debian 10
> container and needs only glibc ≥ 2.28. The `.AppImage`, `.flatpak`, and `.deb` builds
> are alternatives as well. On Wayland you may need to log out/in once after installing
> the icon.

Building from source with `cargo run` on Linux Mint / Ubuntu / Debian requires
the Slint/winit/rfd system development packages:

```bash
sudo apt update
sudo apt install -y --no-install-recommends \
  build-essential pkg-config cmake \
  libfontconfig1-dev libfreetype6-dev \
  libxcb1-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
  libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev \
  libgl1-mesa-dev libegl1-mesa-dev libgtk-3-dev \
  libudev-dev
```

### macOS

The download is a `.zip` containing the `meatshell.app` bundle:

```bash
# Unzip (aarch64 = Apple Silicon, x86_64 = Intel)
unzip meatshell-*-macos-*.zip
# Move it to Applications (optional — it also runs in place)
mv meatshell.app /Applications/
# Clear the quarantine flag, otherwise macOS says "meatshell is damaged and can't be opened"
xattr -dr com.apple.quarantine /Applications/meatshell.app
# Open it (or double-click in Finder)
open /Applications/meatshell.app
```

> If you didn't move it to `/Applications`, point both paths above at wherever the `.app` actually is (e.g. `~/Downloads/meatshell.app`).

> Requires macOS 11 Big Sur or later. Both Apple Silicon and Intel Macs are supported.

> To build from source, see [Running](#running) below.

## Features

### Done

- [x] FinalShell-style UI with dark / light / follow-system themes
- [x] Local + remote resource monitoring (CPU / memory / swap / network / disk)
- [x] Remote process monitor (CPU-sorted table with PID copy and permission-aware termination)
- [x] Full VT/ANSI terminal emulation (btop / htop / vim render correctly)
- [x] Color emoji, including skin tones, flags, and ZWJ sequences
- [x] Tabs (welcome page + multiple sessions)
- [x] Session management: create / edit / delete / groups, local JSON, export / import (including FinalShell connection files)
  - Config location: `%APPDATA%/meatshell/sessions.json` (Windows)
    / `~/.config/meatshell/sessions.json` (Linux)
    / `~/Library/Application Support/meatshell/sessions.json` (macOS)
- [x] SSH (`russh`, pure Rust): password / private key / encrypted key (passphrase)
- [x] SFTP browser + upload / download (drag-and-drop) + in-terminal ZMODEM (`sz` download / `rz` multi-file upload)
- [x] Built-in text viewer / editor: open remote text in the SFTP panel for viewing or editing, with a line-number gutter, find and replace, and save back to the remote host; files above 512 KiB, with too many lines or with an over-long single line are refused with a hint to use an external editor
- [x] WebDAV sync: upload or download the saved session configuration manually (Settings → WebDAV)
- [x] SSH port forwarding / tunnels: local -L / remote -R / dynamic -D (SOCKS5)
- [x] Quick commands + command box (broadcast to all sessions) + command history
- [x] Serial / Telnet sessions
- [x] RDP remote desktop: stores host / port / user / password / domain with a resolution choice (full screen / common sizes / custom) and hands the session to a remote desktop client (`mstsc` on Windows; FreeRDP's `xfreerdp3` / `xfreerdp` on Linux / macOS — install it yourself, or use the Flatpak build, which bundles it)
- [x] Outbound proxy (SOCKS5 / HTTP)
- [x] Import `~/.ssh/config`
- [x] Session passwords encrypted at rest (ChaCha20-Poly1305)
- [x] Known-hosts (`known_hosts`) verification + first-connect confirmation
- [x] Split panes for tabbed terminals
- [x] Multiple windows: Ctrl+Shift+N (macOS ⌘⇧N) or the system "New window" entry (Windows taskbar / Linux desktop right-click), managed as a single Chrome-style process
- [x] MCP activity audit: watch in real time what MCP-capable AI clients do through MCP (see [MCP activity audit](#mcp-activity-audit))

Color emoji graphics are provided by [Twemoji](https://github.com/jdecked/twemoji)
under [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/). See
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) for the full attribution.

### Planned

- [ ] Store session passwords in the OS keychain

## Tech stack

| Module        | Choice                                                            |
| ------------- | ----------------------------------------------------------------- |
| UI            | [Slint](https://slint.dev) (compiled pure Rust, no GC)            |
| Async runtime | [`tokio`](https://tokio.rs)                                       |
| SSH protocol  | [`russh`](https://crates.io/crates/russh) (no libssh dependency)  |
| System metrics| [`sysinfo`](https://crates.io/crates/sysinfo)                     |
| Serialization | `serde` + `serde_json`                                            |
| Logging       | `tracing` + `tracing-subscriber`                                  |

## Running

```bash
cargo run --release
```

On first launch an empty session store is created at
`%APPDATA%/meatshell/sessions.json`. Click **"＋ New Session"** in the top-right
to add your first server.

## CLI and MCP automation

The MeatShell CLI and MCP server share the sessions and SSH/SFTP implementation
used by the GUI. The CLI is suited to scripts, CI, and explicit commands, while
MCP lets an MCP-capable AI client perform server inspection, log analysis, and
file transfers from natural-language requests. They are two entry points to the
same saved server configuration.

> Before using either interface, create the target session in the GUI and connect
> successfully once to complete host-key confirmation. Passwords, private keys,
> and other secrets are never returned by CLI/MCP. Do not put plaintext passwords
> in prompts or MCP configuration files.

### CLI

Show every available command:

```bash
meatshell cli help
```

Common examples:

```bash
# List saved sessions; the first column is the session-id used below.
# A session display name works anywhere a session-id is accepted.
meatshell cli sessions
meatshell cli sessions --json

# Show non-secret metadata for one session
meatshell cli session <session-id-or-name>

# Run a non-interactive SSH command; the remote command must follow --
meatshell cli exec <session-id-or-name> -- free -h
meatshell cli exec <session-id-or-name> --timeout 60 --json -- journalctl -n 100 --no-pager

# Browse, read, and transfer remote files
meatshell cli files <session-id-or-name> /var/log
meatshell cli read <session-id-or-name> /var/log/example.log
meatshell cli upload <session-id-or-name> ./local.txt /tmp
meatshell cli download <session-id-or-name> /tmp/result.txt ./downloads
```

Get `<session-id-or-name>` from `meatshell cli sessions`; either the session id or
its display name works. A download requires an existing local destination directory
and will not overwrite a file with the same name.

### MCP

First open **Settings → Interface → MCP** in MeatShell:

1. Enable MCP.
2. Allow saved credentials when required.
3. Allow arbitrary SSH commands for remote diagnostics.
4. Allow file transfers when uploads or downloads are required.

Then register a stdio MCP server named `meatshell` in your MCP-capable client:

```json
{
  "mcpServers": {
    "meatshell": {
      "command": "/absolute/path/to/meatshell",
      "args": ["mcp", "serve"]
    }
  }
}
```

On Windows, `command` can be `C:\\path\\to\\meatshell.exe`. Restart or refresh
the MCP client; the `meatshell` server should expose tools for session lookup,
remote commands, directory listing, bounded text reads, uploads, and downloads.
MCP configuration locations vary by AI client, so consult that client's docs.

#### MCP tool contract

Every constraint below is stated in each tool's `description`, argument
descriptions, and `readOnlyHint` / `destructiveHint` annotations, so an AI client
reads it directly. It is repeated here so you can check the expected behaviour:

- **Every call is self-contained.** A call connects, does one thing, and
  disconnects; no working directory, shell variable, background job, or
  `~/.bashrc` survives into the next call. Put one job into one command
  (`cd /srv && ./deploy.sh; systemctl status app --no-pager`) and use absolute
  paths for anything outside the default PATH.
- **stdin is closed.** Anything that waits for input — interactive sudo, a
  confirmation prompt, a pager — only burns the timeout. Pass `sudo -n`, `-y`,
  `--no-pager`.
- **The timeout ceiling is 300 seconds.** On timeout the result sets `timed_out`
  with no output, and dropping the connection usually kills the remote command.
  Longer work must be detached from the call (`nohup … >/tmp/job.log 2>&1 &`,
  `setsid`, `systemd-run`) and polled by a later call.
- **`run_command` returns** `stdout`, `stderr`, `exit_code`, `timed_out`,
  `truncated`, plus `exit_signal` when a signal killed the command. A non-zero
  `exit_code` is a normal result, not an error; `exit_code` is null only when the
  command was signalled or the connection went away (rebooting the host,
  restarting sshd). An exec request the server rejects, or a call that returns
  neither output nor exit status, is now an explicit error instead of an empty
  "success".
- **`upload_file` replaces an existing file.** The write is atomic (staged under
  a temp name, then renamed into place) and a replaced file keeps its own
  permissions; a brand-new file lands on the server's default mode, so tighten it
  with `run_command chmod` when the content is sensitive. The source must live
  inside the MCP process working directory with no `..` segments, and the result
  carries the resulting `remote_path`.
- **`download_file` never overwrites.** The local directory must already exist
  and an existing file of the same name fails the call; the result carries
  `local_path`.
- **`read_remote_text_file` is bounded:** 512 KiB, 20000 lines, 64 KiB on any
  single line, UTF-8 only. Use `download_file` for a bigger or binary file, or
  read a slice with `run_command` (`sed -n`, `tail`).
- **Never put a password, token, or private key in a command or a path.** Every
  call is appended to the MCP activity log below; move a secret with
  `upload_file` and then tighten its mode with `run_command`.

#### MCP activity audit

`meatshell mcp serve` (a separate stdio process) appends one JSON line per
invocation to `<data-dir>/mcp_activity.jsonl` — the same data directory as
`sessions.json` (`config/mcp_activity.jsonl` beside the binary in portable mode).
Open **Settings → Interface → MCP** and the panel polls every second to show
"Recent MCP activity" live: timestamp, caller (from the `initialize` handshake's
clientInfo, e.g. `claude-desktop/1.0.0`), tool, command / path, status, and
duration, with Refresh and Clear buttons. Clear renames the old log aside as
`mcp_activity.jsonl.bak` and starts fresh.

Only whitelisted argument fields are recorded (`session_id` / `group` / `command`
/ `path` / `local_path` / `remote_path`, …), inline secrets such as tokens and
passwords in commands are redacted first, and plaintext keys never reach the log.
The file is capped at about 2 MiB, keeping only the newest 1000 records.

#### MCP JSON-RPC examples

An AI client normally creates these requests automatically; you do not need to
enter them manually. When debugging the stdio connection, send each request as
one complete line of JSON and complete initialization in this order:

```jsonl
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"example-client","version":"1.0.0"}}}
{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}
{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}
```

List saved sessions and obtain a `<session-id-or-name>`:

```jsonl
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_sessions","arguments":{}}}
{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"get_session","arguments":{"session_id":"<session-id-or-name>"}}}
```

Run read-only OOM diagnostics and browse the heap-dump directory:

```jsonl
{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"run_command","arguments":{"session_id":"<session-id-or-name>","command":"free -h; printf '\\n=== kernel OOM ===\\n'; dmesg 2>/dev/null | grep -iE 'oom|out of memory|killed process' | tail -50 || true; printf '\\n=== Java ===\\n'; ps -ef | grep '[j]ava'","timeout_seconds":30,"max_output_bytes":1048576}}}
{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"list_remote_files","arguments":{"session_id":"<session-id-or-name>","path":"/home/jeff/test/heapdumps"}}}
```

Read a log or download a heap dump:

```jsonl
{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"read_remote_text_file","arguments":{"session_id":"<session-id-or-name>","path":"/home/jeff/test/logs/meatshell-log-demo-error.log"}}}
{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"download_file","arguments":{"session_id":"<session-id-or-name>","remote_path":"/home/jeff/test/heapdumps/example.hprof","local_directory":"/existing/local/directory","timeout_seconds":120}}}
```

`read_remote_text_file` accepts only bounded UTF-8 text. Use `download_file` for
binary files such as HPROF dumps. The local destination directory must already
exist, and the tool will not overwrite a file with the same name.

Once configured, give the AI client a request such as:

> Use the `meatshell` MCP to investigate an OOM on my `192.168.100.41` server.
> Heap dumps are in `/home/jeff/test/heapdumps`. Check system memory, kernel OOM
> records, Java processes, application logs, and the HPROF files, then identify
> the root cause. Perform read-only diagnostics first; do not restart services or
> delete files.

MCP first uses `list_sessions` to find the matching saved session, then invokes
remote-command or SFTP tools within the permissions you granted. Every tool's
`session_id` argument also accepts the session's display name — pass the name
directly to locate and operate a server without looking up its id; an ambiguous
name returns an error listing the candidate ids. If several sessions use the same
host, include the GUI session name in the prompt. A good diagnostic prompt states
the target host, log or dump paths, and whether restarts, configuration changes,
or file downloads are allowed.

## Project layout

```
meatshell/
├── Cargo.toml
├── build.rs                      # Slint compiler entry point
├── ui/                           # Slint UI (declarative)
│   ├── app.slint                 # top-level window and pages
│   ├── interface_panel.slint     # settings panel (interface / MCP / WebDAV / update check…)
│   ├── sftp_panel.slint          # SFTP file panel and transfer queue
│   ├── sidebar.slint             # left-hand local system monitor
│   ├── system_info_window.slint  # system information window
│   ├── proc_window.slint         # process list window
│   ├── tabs.slint                # top tab bar and split panes
│   ├── welcome.slint             # welcome page / quick connect
│   ├── session_dialog.slint      # new / edit session dialog
│   ├── confirm_dialog.slint      # confirmation dialog
│   ├── terminal_view.slint       # terminal view
│   ├── theme.slint               # design tokens
│   └── widgets.slint             # reusable buttons / inputs / sparkline
├── packaging/                    # AUR (`PKGBUILD`) and Flatpak manifests
└── src/
    ├── main.rs                   # entry point: GUI / `cli` / `mcp serve`
    ├── app.rs + app/             # UI ↔ backend bridge (windows, tabs, sessions, terminal, SFTP, WebDAV…)
    ├── ui/                       # Slint module entry (`include_modules!`)
    ├── config/                   # session / quick-command / credential persistence, FinalShell import
    ├── session/                  # connection context, pending host key / credential / MFA state
    ├── ssh/                      # SSH client (russh, known_hosts, outbound proxy, PPK)
    ├── sftp/                     # SFTP (browse / upload / download / built-in editor)
    ├── tunnel/                   # port forwarding -L / -R / -D
    ├── terminal/                 # terminal emulation: VT rendering, charsets, local / serial / Telnet, ZMODEM
    ├── webdav/                   # WebDAV sync
    ├── resource/                 # local and remote resource sampling
    ├── layout/                   # terminal split-pane layout
    ├── automation/               # tool implementations shared by CLI and MCP
    ├── cli/                      # command-line subcommands
    ├── mcp/                      # MCP stdio server, tool implementations, activity audit
    ├── i18n/                     # UI strings
    ├── logging/                  # tracing and error logs
    ├── wallpaper/                # background image
    └── allocator/                # platform-selected heap allocator (mimalloc on Windows)
```

## Development notes

- Slint widgets use a strict layout DSL; after editing a `.slint` file,
  `cargo check` is the fastest feedback loop.
- The application event loop is single-threaded (required by Slint); all
  cross-thread UI updates go through `slint::invoke_from_event_loop` callbacks.
- SSH / SFTP share the `known_hosts` verification path: first contact asks for
  trust and remembers the host key, while later key changes prompt again.

## Release

Pushing to `main` already builds and publishes the artifacts as `v<version>-ci-<short-sha>`
(Latest), with the version taken from `Cargo.toml` — so main always has a downloadable
build even without a tag, it just keeps the same version number.

Do not bump `Cargo.toml` by hand and then create a tag. Use the release helper
so the tag points at a commit that already contains the matching Cargo version:

```powershell
.\scripts\release.ps1 v0.6.0 -Push
```

The script updates `Cargo.toml` / `Cargo.lock`, runs `cargo check --locked`,
verifies `meatshell --version`, commits `Release v0.6.0`, creates an annotated
tag, and pushes the current branch plus the tag. See
[docs/release.md](docs/release.md) for details.

## Related Groups

<p align="center">
  <img src="docs/QR/QQ_Group_QR_Code.jpg" alt="QQ group QR code" width="300"><br>
  <em>Scan the QR code to join QQ groups to exchange user experiences, provide feedback, or get the latest updates</em>
</p>

## License

Dual-licensed under MIT OR Apache-2.0.
