use std::collections::HashSet;
use anyhow::{bail, Result};
use super::{ConfigStore, Session, SessionKind};

/// Bound recursive connection/authentication work even for malformed imports.
const MAX_JUMP_HOPS: usize = 16;

impl ConfigStore {
    /// Immediate jump first; following entries are that jump's ancestors.
    /// Resolve the full graph before any network activity, never silently direct.
    pub fn resolve_jump_chain(&self, target: &Session) -> Result<Vec<Session>> {
        resolve_jump_chain(self.sessions(), target)
    }
}

fn resolve_jump_chain(sessions: &[Session], target: &Session) -> Result<Vec<Session>> {
    if target.kind != SessionKind::Ssh || target.jump_session_id.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut visited = HashSet::from([target.id.as_str()]);
    let mut next = target.jump_session_id.as_str();
    let mut chain = Vec::new();
    while !next.trim().is_empty() {
        if !visited.insert(next) {
            bail!("SSH jump chain contains a cycle at session {next}");
        }
        if chain.len() >= MAX_JUMP_HOPS {
            bail!("SSH jump chain exceeds {MAX_JUMP_HOPS} hops");
        }
        let hop = sessions.iter().find(|s| s.id == next)
            .ok_or_else(|| anyhow::anyhow!("SSH jump session not found: {next}"))?;
        if hop.kind != SessionKind::Ssh {
            bail!("SSH jump session must use SSH: {}", hop.id);
        }
        chain.push(hop.clone());
        next = hop.jump_session_id.as_str();
    }
    Ok(chain)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session(id: &str, jump: &str) -> Session {
        let mut s = Session::new_empty();
        s.id = id.into();
        s.jump_session_id = jump.into();
        s
    }
    #[test]
    fn direct_and_single_hop() {
        let direct = session("direct", "");
        assert!(resolve_jump_chain(&[], &direct).unwrap().is_empty());
        let target = session("target", "direct");
        assert_eq!(resolve_jump_chain(&[direct], &target).unwrap()[0].id, "direct");
    }
    #[test]
    fn nested_hops_preserve_order_and_individual_credentials() {
        let target = session("target", "inner");
        let mut inner = session("inner", "outer");
        inner.user = "inside".into();
        let mut outer = session("outer", "");
        outer.user = "outside".into();
        let hops = resolve_jump_chain(&[outer, inner], &target).unwrap();
        assert_eq!(hops.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["inner", "outer"]);
        assert_eq!(hops.iter().map(|s| s.user.as_str()).collect::<Vec<_>>(), ["inside", "outside"]);
    }
    #[test]
    fn missing_inner_ancestor_fails_instead_of_direct_connect() {
        let target = session("target", "inner");
        assert!(resolve_jump_chain(&[], &target).is_err());
        let err = resolve_jump_chain(&[session("inner", "missing")], &target).unwrap_err();
        assert!(err.to_string().contains("missing"));
    }
    #[test]
    fn self_reference_and_long_cycles_are_rejected() {
        let target = session("target", "target");
        assert!(resolve_jump_chain(&[], &target).unwrap_err().to_string().contains("cycle"));
        let target = session("target", "a");
        let saved = [session("a", "b"), session("b", "a")];
        assert!(resolve_jump_chain(&saved, &target).unwrap_err().to_string().contains("cycle"));
    }
    #[test]
    fn non_ssh_hop_is_rejected() {
        let mut hop = session("serial", "");
        hop.kind = SessionKind::Serial;
        assert!(resolve_jump_chain(&[hop], &session("target", "serial")).is_err());
    }
    #[test]
    fn maximum_depth_is_bounded() {
        let mut saved: Vec<Session> = (0..MAX_JUMP_HOPS)
            .map(|i| session(&format!("h{i}"), if i == 0 { "" } else { "unused" }))
            .collect();
        for i in 1..saved.len() { saved[i].jump_session_id = format!("h{}", i - 1); }
        let target = session("target", &format!("h{}", MAX_JUMP_HOPS - 1));
        assert_eq!(resolve_jump_chain(&saved, &target).unwrap().len(), MAX_JUMP_HOPS);
        saved.push(target);
        let err = resolve_jump_chain(&saved, &session("extra", "target")).unwrap_err();
        assert!(err.to_string().contains("exceeds"));
    }
}
