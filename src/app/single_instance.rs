// Single-instance coordination. All OS entry points (Windows jump list,
// macOS dock menu, Linux desktop action) launch `meatshell --new-window`.
// The first running instance owns a local endpoint under the data dir; later
// `--new-window` launches connect, send "new-window\n" and exit, and the
// primary opens the new window in-process (Chrome-style). Plain relaunches
// never forward: if the endpoint is taken they run as an independent second
// instance.
//
// Transport split: unix uses a unix-domain socket (`ipc.sock`); Windows uses
// a TCP loopback listener on 127.0.0.1 whose port is published in a port
// file (`ipc.port`), because std's Windows unix-socket support is unstable
// (nightly-only, rust-lang/rust#150487). On Windows every `socket_path`
// argument is therefore reinterpreted as the port-file path.
//
// Security (#12): the endpoint file and a per-run random token file are
// created with owner-only (0600) permissions, and every `--new-window`
// forward carries the token. The listener only acks a request whose token
// matches, so a forged listener (or a stale port file pointed at one) cannot
// impersonate the primary. Old-protocol forwarders (no token) are still
// accepted for backwards compatibility with a previous build still running.

#[cfg(windows)]
use std::net::{SocketAddr, TcpListener, TcpStream};
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

const MSG_NEW_WINDOW: &str = "new-window";

#[derive(Debug)]
pub enum Instance {
    /// This process owns the endpoint. `listen` accepts forwarded requests.
    Primary { listen: Listener },
    /// Another instance is running; the new-window request was forwarded and
    /// this process should exit with success.
    Forwarded,
}

/// Try to become the primary instance. With `forward`, a live primary
/// receives a new-window request and `Forwarded` is returned; without it a
/// live primary is reported as an error so a plain relaunch runs as its own
/// instance instead of opening a bonus window in the first one. Never
/// panics on IO trouble — callers treat errors as "just run normally".
#[cfg(unix)]
pub fn acquire(socket_path: &Path, forward: bool) -> std::io::Result<Instance> {
    let token_file = token_path_for(socket_path);
    if let Ok(listener) = UnixListener::bind(socket_path) {
        return register_primary(listener, socket_path, &token_file);
    }
    // Bind failed: either a live primary owns the socket, or the file is
    // stale (the previous primary exited without unlinking it). Probe with
    // a connect, mirroring the stale port-file logic on Windows.
    match UnixStream::connect(socket_path) {
        Ok(mut stream) => {
            if !forward {
                // A plain launch must not forward: report the live primary
                // and let run() fall back to a normal second instance.
                return Err(std::io::Error::other(
                    "single-instance primary already running",
                ));
            }
            stream.set_read_timeout(Some(Duration::from_secs(2)))?;
            forward_stream(&mut stream, &token_file)
        }
        Err(_) => {
            // Nothing is listening, so the file is stale. Remove it and
            // retry the bind once. Shutdown-time unlinking is deliberately
            // not attempted: it cannot run for a killed process, so this
            // retry is the robust recovery.
            let _ = std::fs::remove_file(socket_path);
            let listener = UnixListener::bind(socket_path)?;
            register_primary(listener, socket_path, &token_file)
        }
    }
}

#[cfg(unix)]
fn register_primary(
    listener: UnixListener,
    socket_path: &Path,
    token_file: &Path,
) -> std::io::Result<Instance> {
    use std::os::unix::fs::PermissionsExt;
    // Restrict the socket and token file to the owning user so other local
    // users can neither connect nor learn the handshake token (#12).
    let _ = std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o600));
    let token = random_token();
    write_private_file(token_file, &token.to_string())?;
    Ok(Instance::Primary {
        listen: Listener {
            listener,
            token: Some(token),
        },
    })
}

/// `socket_path` is the port-file path here (see module docs).
#[cfg(windows)]
pub fn acquire(socket_path: &Path, forward: bool) -> std::io::Result<Instance> {
    let port_file = socket_path;
    let token_file = token_path_for(socket_path);
    // A live primary? Connect with a short timeout; a stale port file
    // (parse error, refused/timeout connection) is treated as "no primary".
    if let Some(port) = read_port_file(port_file) {
        if forward {
            if forward_tcp(port, &token_file).is_ok() {
                return Ok(Instance::Forwarded);
            }
        } else if connect_tcp(port).is_ok() {
            // A plain launch must not forward (and must not steal the port
            // file): report the live primary and let run() fall back to a
            // normal second instance.
            return Err(std::io::Error::other(
                "single-instance primary already running",
            ));
        }
    }
    // No live primary: start one on an ephemeral loopback port.
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    // Without the port file nobody can find us; let the caller fall back
    // to a normal launch.
    write_private_file(port_file, &port.to_string())?;
    // Two processes can both see "no primary" and both reach this point.
    // Whoever wrote the port file last wins; re-check and defer if we lost.
    if read_port_file(port_file) != Some(port) {
        drop(listener);
        if forward {
            if let Some(winner) = read_port_file(port_file) {
                if forward_tcp(winner, &token_file).is_ok() {
                    return Ok(Instance::Forwarded);
                }
            }
        }
        // Degradation: the race-loser could not reach the winner, so a
        // `--new-window` launch falls back to a second instance; rare and
        // acceptable.
        return Err(std::io::Error::other("lost single-instance race"));
    }
    // We won the race: register the token now so a forwarder can prove we
    // are the real primary (writing it earlier would let the loser's token
    // clobber ours).
    let token = random_token();
    write_private_file(&token_file, &token.to_string())?;
    Ok(Instance::Primary {
        listen: Listener {
            listener,
            token: Some(token),
        },
    })
}

#[cfg(windows)]
fn connect_tcp(port: u16) -> std::io::Result<TcpStream> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(300))
}

#[cfg(windows)]
fn forward_tcp(port: u16, token_file: &Path) -> std::io::Result<Instance> {
    let mut stream = connect_tcp(port)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    forward_stream(&mut stream, token_file)
}

/// Write the new-window request (with the handshake token when one is
/// present) and verify the primary's ack. A missing token file means the
/// primary is a pre-token build: fall back to the plain protocol.
fn forward_stream<S: std::io::Read + std::io::Write>(
    stream: &mut S,
    token_file: &Path,
) -> std::io::Result<Instance> {
    let token = read_token_file(token_file);
    let request = match token {
        Some(t) => format!("{MSG_NEW_WINDOW} {t}\n"),
        None => format!("{MSG_NEW_WINDOW}\n"),
    };
    stream.write_all(request.as_bytes())?;
    stream.flush()?;
    // Whoever holds the port must ack: a stale port file can point at an
    // unrelated listener, and treating that as Forwarded would exit without
    // any window ever opening. A token-bearing request additionally requires
    // the primary to echo the token, so a forged listener (which cannot read
    // the 0600 token file) cannot impersonate it (#12).
    let expected = match token {
        Some(t) => format!("ack {t}"),
        None => "ack".to_string(),
    };
    match BufReader::new(stream).lines().next() {
        Some(Ok(line)) if line == expected => Ok(Instance::Forwarded),
        _ => Err(std::io::Error::other("no ack from single-instance primary")),
    }
}

#[cfg(windows)]
fn read_port_file(port_file: &Path) -> Option<u16> {
    std::fs::read_to_string(port_file).ok()?.trim().parse().ok()
}

/// Endpoint path inside the per-user data dir: the unix socket on unix, the
/// TCP port file on Windows (see module docs).
pub fn socket_path() -> PathBuf {
    #[cfg(windows)]
    {
        crate::config::data_dir().join("ipc.port")
    }
    #[cfg(not(windows))]
    {
        crate::config::data_dir().join("ipc.sock")
    }
}

#[derive(Debug)]
pub struct Listener {
    #[cfg(unix)]
    listener: UnixListener,
    #[cfg(windows)]
    listener: TcpListener,
    /// Per-run random token the primary authenticates forwards with (#12).
    token: Option<u64>,
}

impl Listener {
    /// Blocks forever, invoking `on_msg` for every complete line received.
    /// Spawn this on its own thread. Every accepted line is acked before
    /// dispatch so forwarders can verify they reached the real primary —
    /// on Windows the port file may stale and point at an unrelated
    /// listener, which would never ack.
    pub fn spawn<F: FnMut(String) + Send + 'static>(self, mut on_msg: F) {
        let token = self.token;
        for mut stream in self.listener.incoming().flatten() {
            // One silent client must not wedge every later forward: bound
            // the wait for the first line and skip the connection if it
            // times out.
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            if let Some(Ok(line)) = BufReader::new(&stream).lines().next() {
                // Authenticate: accept the bare old-protocol request, or a
                // token-bearing one whose token matches ours. Anything else
                // is rejected without an ack so a forged listener can't be
                // confirmed by a forwarder.
                let (ack, dispatch) = match token {
                    Some(expect) => {
                        if line == MSG_NEW_WINDOW {
                            ("ack\n".to_string(), MSG_NEW_WINDOW.to_string())
                        } else if let Some(given) =
                            line.strip_prefix(&format!("{MSG_NEW_WINDOW} "))
                        {
                            match given.trim().parse::<u64>() {
                                Ok(g) if g == expect => {
                                    (format!("ack {expect}\n"), MSG_NEW_WINDOW.to_string())
                                }
                                _ => continue,
                            }
                        } else {
                            continue;
                        }
                    }
                    None => ("ack\n".to_string(), line.clone()),
                };
                let _ = stream.write_all(ack.as_bytes());
                let _ = stream.flush();
                on_msg(dispatch);
            }
        }
    }
}

/// The handshake token file lives next to the endpoint (port/socket) file
/// and is named after it so multiple endpoints in one directory don't
/// collide. Old binaries never create it, so its absence marks an old
/// primary and the forward falls back to the plain protocol.
fn token_path_for(endpoint: &Path) -> PathBuf {
    let mut name = endpoint.file_name().unwrap_or_default().to_os_string();
    name.push(".token");
    endpoint.with_file_name(name)
}

fn random_token() -> u64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    // RandomState is seeded from OS entropy per call; fold in the pid and a
    // timestamp so even a zeroed seed can't collide across processes.
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(std::process::id().into());
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
    );
    hasher.finish()
}

fn read_token_file(path: &Path) -> Option<u64> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// Write a file with owner-only (0600) permissions. On Windows std exposes no
/// stable owner-mode setter (windows_permissions_ext is nightly-only), so the
/// mode is skipped there; the per-user data-dir ACL plus the handshake token
/// provide the actual protection.
fn write_private_file(path: &Path, contents: &str) -> std::io::Result<()> {
    std::fs::write(path, contents)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/app/window_management/single_instance.rs"]
mod single_instance_tests;
