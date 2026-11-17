//! The RX 9070 XT's admission as a Metal device on the Linux host (J2, J4):
//! the AMD route this tool measured before Stela's own AMD stack existed,
//! kept here as the host tool's, since the image's stack (`metal::amd`)
//! is built only with the amdgpu port.
//!
//! The Metal device opens through `metal::open` on a [`GpuStack`] whose
//! admission is [`Chip::admit`]: `Gpu::chip` carries the graphics core
//! ([`encode`]), which the host reads from sysfs or RADV's device name.
use crate::chip::{CHIPS, Chip, FIRST_SUPPORTED_GC, GcVersion};
use stela_driver_gpu::{
    Vendor,
    metal::{
        abi::DeviceLocation,
        capabilities::Platform,
        nvidia,
        open::{self, Admission, Attachment},
        vulkan::{DriverId, PhysicalDeviceType, physical::Identity},
    },
    stack::{GpuStack, Kernel},
};

pub const VENDOR_ID: u32 = 0x1002;

/// Why a Vulkan device is not admitted as an AMD Metal device.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Refusal {
    NotAmd {
        vendor_id: u32,
    },
    /// Only RADV backs the AMD route.
    NotRadv(DriverId),
    /// Graphics cores before RDNA 4 are not this tool's.
    PreRdna4 {
        gc: GcVersion,
    },
    /// A core the table does not name.
    UnknownCore {
        gc: GcVersion,
    },
    /// APUs are not this tool's.
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

/// Row-pitch alignment for explicitly laid out linear images, measured on
/// the RX 9070 XT (RADV GFX1201, `DRM_FORMAT_MOD_LINEAR` with an explicit
/// layout): every multiple of 128 at least a row's bytes is accepted, and
/// no other pitch. The image's stack answers 256, which every generation
/// takes.
const LINEAR_ROW_ALIGNMENT: u64 = 128;
/// Offset alignment for binding a linear image inside a buffer, measured on
/// the same card: 256 bytes.
const LINEAR_OFFSET_ALIGNMENT: u64 = 256;
/// RADV's sparse binding granularity, the 64 KiB page.
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

/// `gc` as `Gpu::chip`, which only [`HOST_AMD`]'s admission reads.
pub fn encode(gc: GcVersion) -> u32 {
    u32::from(gc.major) << 16 | u32::from(gc.minor) << 8 | u32::from(gc.revision)
}

fn decode(chip: u32) -> GcVersion {
    GcVersion::new((chip >> 16) as u8, (chip >> 8) as u8, chip as u8)
}

fn admit(
    identity: &Identity,
    chip: u32,
    attachment: Attachment,
) -> Result<Platform, open::Refusal> {
    Chip::admit(identity, decode(chip))
        .map(|chip| chip.platform(attachment))
        // `open::Refusal` names AMD only with the amdgpu port; the tool has
        // admitted the device before it opens it ([`Chip::admit`]).
        .map_err(|_| {
            open::Refusal::Nvidia(nvidia::Refusal::NotNvidia {
                vendor_id: identity.vendor_id,
            })
        })
}

/// The host's AMD stack: amdgpu as the host's kernel runs it, and the
/// admission above. Only its admission is read on the host.
pub static HOST_AMD: GpuStack = GpuStack {
    vendor: Vendor::Amd,
    kernel: Kernel {
        driver: "amdgpu",
        admits: |_| Ok(()),
        registers: 5,
        dma_bits: |_| 44,
        integrated: |_| false,
        probed_integrated: None,
        chip: |_| None,
        memory_bytes: |_| None,
        reset: None,
    },
    metal: Some(Admission { admit }),
    // The tool loads RADV itself (`device.rs`), not through the stack.
    #[cfg(feature = "radv-host")]
    vulkan: None,
};

/// The graphics core RADV names a device by, e.g. "(RADV GFX1201)", for
/// hosts whose kernel publishes no IP discovery table.
pub fn core_from_name(name: &str) -> Option<GcVersion> {
    CHIPS
        .iter()
        .find(|chip| name.contains(&format!("RADV {}", chip.name)))
        .map(|chip| chip.gc)
}
