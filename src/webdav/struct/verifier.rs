/// Custom rustls verifier that accepts *any* TLS certificate.
///
/// SECURITY (#32): this is fail-open by design and MUST NOT be used as the
/// default verifier — it would let a MITM read WebDAV credentials. It is only
/// injected by `webdav_agent` when the user explicitly enables "trust
/// self-signed / intranet certs" (`accept_invalid_certs`); the default path
/// uses rustls' built-in verifier (system trust store + ServerName hostname
/// check) and stays fail-closed. Keep it out of any default TLS configuration.
#[derive(Debug)]
pub(crate) struct WebDavAcceptAnyCertVerifier;
