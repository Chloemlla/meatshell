#[path = "struct/resource_struct.rs"]
mod resource_struct;
#[path = "impls/system_impl.rs"]
pub(crate) mod system;

pub(crate) use resource_struct::{
    LocalGpuInfo, LocalHardwareInfo, LocalSnap, NetHist, TabStatus, TabStatuses,
};
