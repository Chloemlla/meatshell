#[path = "impls/certificate_verifier.rs"]
mod certificate_verifier;
#[path = "struct/types.rs"]
mod types;

pub(crate) use types::WebDavAcceptAnyCertVerifier;
