#[path = "impls/system.rs"]
pub(crate) mod system;
#[path = "struct/types.rs"]
mod types;

pub(crate) use types::{
    LocalGpuInfo, LocalHardwareInfo, LocalSnap, NetHist, TabStatus, TabStatuses,
};
