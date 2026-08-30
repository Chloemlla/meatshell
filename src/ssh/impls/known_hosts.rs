//! Host-key verification store (#109-5 / #105).
//!
//! Replaces the old "accept any server key" behaviour with a TOFU-style
//! known_hosts file plus a first-connect confirmation dialog:
//!   • unknown host  → prompt the user with the key fingerprint; on accept the
//!                     key is remembered here.
//!   • known + match  → connect silently.
//!   • known + differ → flagged as *changed* (possible MITM); the user must
//!                     re-confirm before the new key replaces the stored one.
//!
//! The file lives next to `sessions.json` (one entry per line):
//!     `host:port ssh-ed25519 AAAA...`
//! i.e. the `host:port` id followed by the key in its OpenSSH one-line form.

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use anyhow::{bail, Context, Result};
use ssh_key::{HashAlg, PublicKey};
use super::structs::HostKeyStatus;

/// Serializes `remember()`'s read-modify-write cycles so two sessions that
/// finish their first host-key confirmation at the same time can't overwrite
/// each other's just-appended entries (#27).
static KNOWN_HOSTS_LOCK: Mutex<()> = Mutex::new(());

/// Uniquifies the atomic-replace temp name within this process (combined with
/// the PID it is unique across processes too), so a stale temp file from a
/// crashed run can't collide with a live one (#27).
static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// `host:port` lookup key.
fn id(host: &str, port: u16) -> String {
    format!("{host}:{port}")
}

/// Path to the known_hosts file (alongside sessions.json, in the portable-first
/// data dir — #141).
fn path() -> Option<PathBuf> {
    Some(crate::config::data_dir().join("known_hosts"))
}

/// The presented key in its canonical OpenSSH one-line form (`type base64`,
/// no comment), used for exact comparison and for storage.
fn openssh_line(key: &PublicKey) -> String {
    // `to_openssh` only fails on an unsupported/!encodable key, which russh
    // would not have negotiated; fall back to the SHA256 fingerprint so a
    // freak case still stores *something* stable rather than panicking.
    key.to_openssh().unwrap_or_else(|_| fingerprint(key))
}

/// Human-readable SHA256 fingerprint (`SHA256:base64…`) shown in the dialog.
pub fn fingerprint(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

/// Parse the file into `(id, openssh_key)` entries. Missing file → empty.
/// Malformed / comment (`#`) lines are skipped.
fn load() -> Vec<(String, String)> {
    let Some(p) = path() else { return Vec::new() };
    let Ok(text) = std::fs::read_to_string(&p) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (id, key) = line.split_once(char::is_whitespace)?;
            Some((id.to_string(), key.trim().to_string()))
        })
        .collect()
}

/// Check a presented server key against the store.
pub fn verify(host: &str, port: u16, key: &PublicKey) -> HostKeyStatus {
    let want = openssh_line(key);
    let id = id(host, port);
    let mut seen_host = false;
    for (entry_id, entry_key) in load() {
        if entry_id != id {
            continue;
        }
        seen_host = true;
        if entry_key == want {
            return HostKeyStatus::Match;
        }
    }
    if seen_host {
        HostKeyStatus::Changed
    } else {
        HostKeyStatus::Unknown
    }
}

/// Remember (or replace) the key for `host:port`. Rewrites the file with any
/// stale entry for the same id removed, then appends the new one.
///
/// The rewrite is serialized process-wide and applied by writing a unique temp
/// file in the same directory then atomically renaming it over the target, so a
/// crash mid-write never tears the file and concurrent first-confirmations can't
/// clobber each other (#27). Before writing we refuse a pre-planted symlink or
/// directory at the target path, so a malicious symlink can't redirect the write
/// (TOFU takeover, #28).
pub fn remember(host: &str, port: u16, key: &PublicKey) -> Result<()> {
    let p = path().context("could not determine config directory")?;
    let parent = p.parent().context("known_hosts path has no parent directory")?;
    std::fs::create_dir_all(parent).context("create config dir")?;

    // One writer at a time: the rewrite below is read-modify-write.
    let _guard = KNOWN_HOSTS_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    // Refuse to follow a pre-planted symlink / directory (TOFU takeover, #28).
    match std::fs::symlink_metadata(&p) {
        Ok(md) => {
            if md.file_type().is_symlink() {
                bail!(
                    "refusing to write known_hosts through a symbolic link: {}",
                    p.display()
                );
            }
            if md.is_dir() {
                bail!("known_hosts path is a directory: {}", p.display());
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).with_context(|| format!("stat {}", p.display())),
    }

    let id = id(host, port);
    let line = openssh_line(key);
    let mut out = String::new();
    for (entry_id, entry_key) in load() {
        if entry_id == id {
            continue; // drop the old key for this host:port
        }
        out.push_str(&entry_id);
        out.push(' ');
        out.push_str(&entry_key);
        out.push('\n');
    }
    out.push_str(&id);
    out.push(' ');
    out.push_str(&line);
    out.push('\n');

    // Atomic replace: write + fsync a unique temp file, then rename over the
    // target (#27). The target is replaced atomically, never followed.
    let tmp = parent.join(format!(
        "known_hosts.tmp.{}.{}",
        std::process::id(),
        TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| -> Result<()> {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts
            .open(&tmp)
            .with_context(|| format!("create {}", tmp.display()))?;
        f.write_all(out.as_bytes())
            .with_context(|| format!("write {}", tmp.display()))?;
        f.sync_all()
            .with_context(|| format!("sync {}", tmp.display()))?;
        drop(f);
        std::fs::rename(&tmp, &p)
            .with_context(|| format!("rename {} -> {}", tmp.display(), p.display()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}
