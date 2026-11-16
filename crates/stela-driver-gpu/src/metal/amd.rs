//! AMD Radeon GPUs under Stela's Metal device: amdgpu in the kernel and
//! RADV as the Vulkan driver.
//!
//! This is the AMD counterpart of [`super::nvidia`]. Hardware access,
//! firmware, memory management, command submission and shader code
//! generation are meant to stay with amdgpu, RADV and ACO; this module only
//! admits a device and supplies the facts Vulkan does not report, and
//! [`open`] serves it.
//!
//! Neither amdgpu nor RADV is part of the boot image yet, so nothing reaches
//! this route on hardware: it is exercised by the tests over the mock
//! backend, ready for the kernel and driver ports.
use super::{
    abi::DeviceLocation,
    capabilities::Platform,
    vulkan::{DriverId, PhysicalDeviceType, physical::Identity},
};
use alloc::string::String;

pub mod open;

pub const VENDOR_ID: u32 = 0x1002;

pub use super::nvidia::{Attachment, PcieLink};
pub use crate::amdgpu_chip::{Architecture, CHIPS, Chip, FIRST_SUPPORTED_GC, GcVersion};

/// Why a Vulkan device is not admitted as an AMD Metal device.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Refusal {
    NotAmd {
        vendor_id: u32,
    },
    /// Only RADV is meant to sit on Stela's amdgpu winsys.
    NotRadv(DriverId),
    /// Graphics cores before RDNA 4 are not admitted yet.
    PreRdna4 {
        gc: GcVersion,
    },
    /// amdgpu's discovery reported a core this table does not name.
    UnknownCore {
        gc: GcVersion,
    },
    /// APUs share system memory and are not admitted yet.
    NotDiscrete(PhysicalDeviceType),
}

impl Chip {
    /// Admits the Vulkan device `identity` whose kernel driver reports the
    /// graphics core `gc`.
    pub fn admit(identity: &Identity, gc: GcVersion) -> Result<&'static Chip, Refusal> {
        if identity.vendor_id != VENDOR_ID {
            return Err(Refusal::NotAmd {
                vendor_id: identity.vendor_id,
            });
        }
        if identity.driver_id != DriverId::MesaRadv {
            return Err(Refusal::NotRadv(identity.driver_id));
        }
        if identity.device_type != PhysicalDeviceType::DiscreteGpu {
            return Err(Refusal::NotDiscrete(identity.device_type));
        }
        if gc < FIRST_SUPPORTED_GC {
            return Err(Refusal::PreRdna4 { gc });
        }
        Self::find(gc).ok_or(Refusal::UnknownCore { gc })
    }
}

/// Row-pitch alignment for explicitly laid out linear images: addrlib's 128
/// bytes for linear surfaces on GFX12 (256 on GFX9-11, which Stela does not
/// admit). Measured on the RX 9070 XT (RADV GFX1201, `DRM_FORMAT_MOD_LINEAR`
/// with an explicit layout): every multiple of 128 at least a row's bytes
/// is accepted, and no other pitch.
const LINEAR_ROW_ALIGNMENT: u64 = 128;
/// Offset alignment for binding a linear image inside a buffer. Measured on
/// the same card: the image's memory alignment and the plane offsets RADV
/// accepts are multiples of 256 bytes.
const LINEAR_OFFSET_ALIGNMENT: u64 = 256;
/// RADV's sparse binding granularity is the 64 KiB page, matching Vulkan's
/// standard sparse block shapes.
const SPARSE_PAGE_BYTES: u64 = 64 << 10;

impl Chip {
    pub fn platform(&self, attachment: Attachment) -> Platform {
        Platform {
            architecture: String::from(self.name),
            location: if attachment.external {
                DeviceLocation::External
            } else {
                DeviceLocation::Slot
            },
            location_number: attachment.location_number,
            max_transfer_rate: attachment.link.bytes_per_second(),
            headless: !self.display,
            removable: attachment.external,
            linear_row_alignment: LINEAR_ROW_ALIGNMENT,
            linear_offset_alignment: LINEAR_OFFSET_ALIGNMENT,
            sparse_page_bytes: SPARSE_PAGE_BYTES,
        }
    }
}

#[cfg(test)]
mod tests;
