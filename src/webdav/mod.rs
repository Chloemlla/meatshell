#[path = "impls/certificate_verifier_impl.rs"]
mod certificate_verifier_impl;
#[path = "struct/webdav_struct.rs"]
mod webdav_struct;

pub(crate) use webdav_struct::WebDavAcceptAnyCertVerifier;
