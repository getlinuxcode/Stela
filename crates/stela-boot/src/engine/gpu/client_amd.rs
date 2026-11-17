//! AMD fork (J7): what the boot asks amdgpu of a bound Radeon, through its
//! render node as [`super::chipset`] asks nouveau: the graphics core's
//! version, which admits the GPU to the Metal device (`metal::amd`), and
//! its VRAM, for the guest's `StelaHost`.
use super::{Operation, RENDER_MINOR, close_descriptor, submit};
use core::ffi::c_int;
use stela_drivers::gpu::amdgpu_chip::GcVersion;

/// `struct drm_amdgpu_info`, with its query's union as its largest member.
#[repr(C)]
#[derive(Default)]
struct Info {
    return_pointer: u64,
    return_size: u32,
    query: u32,
    union: [u32; 4],
}

/// `DRM_IOCTL_AMDGPU_INFO`: `DRM_IOW(DRM_COMMAND_BASE + 0x05, struct drm_amdgpu_info)`.
const INFO: u32 = 1 << 30 | (size_of::<Info>() as u32) << 16 | 0x64 << 8 | 0x45;
/// `AMDGPU_INFO_HW_IP_INFO`, with `AMDGPU_HW_IP_GFX` as its type.
const HW_IP_INFO: u32 = 0x02;
const HW_IP_GFX: u32 = 0;
/// `AMDGPU_INFO_VRAM_GTT`.
const VRAM_GTT: u32 = 0x14;

/// `struct drm_amdgpu_info_hw_ip`.
#[repr(C)]
#[derive(Default)]
struct HwIp {
    version_major: u32,
    version_minor: u32,
    capabilities: u64,
    ib_start_alignment: u32,
    ib_size_alignment: u32,
    available_rings: u32,
    /// Bits 23:16 major, 15:8 minor, 7:0 revision.
    ip_discovery_version: u32,
}

/// `struct drm_amdgpu_info_vram_gtt`.
#[repr(C)]
#[derive(Default)]
struct VramGtt {
    vram_size: u64,
    vram_cpu_accessible_size: u64,
    gtt_size: u64,
}

/// One `AMDGPU_INFO` query of the bound GPU at `index` into `answer`.
fn query<T>(index: u32, query: u32, union: [u32; 4], answer: &mut T) -> bool {
    let fd = submit(Operation::Open(RENDER_MINOR + index)) as c_int;
    if fd < 0 {
        return false;
    }
    let mut info = Info {
        return_pointer: (answer as *mut T) as u64,
        return_size: size_of::<T>() as u32,
        query,
        union,
    };
    let result = submit(Operation::Control {
        fd,
        request: INFO,
        argument: (&raw mut info).cast(),
    });
    close_descriptor(fd);
    result == 0
}

/// The graphics core version (GC) of the bound Radeon at `index`, as its IP
/// discovery table gave it to amdgpu.
pub(in crate::engine) fn gc_version(index: u32) -> Option<GcVersion> {
    let mut ip = HwIp::default();
    if !query(index, HW_IP_INFO, [HW_IP_GFX, 0, 0, 0], &mut ip) || ip.ip_discovery_version == 0 {
        return None;
    }
    let version = ip.ip_discovery_version;
    Some(GcVersion::new(
        (version >> 16) as u8,
        (version >> 8) as u8,
        version as u8,
    ))
}

/// The VRAM bytes of the bound Radeon at `index`.
pub(in crate::engine) fn vram_bytes(index: u32) -> Option<u64> {
    let mut sizes = VramGtt::default();
    (query(index, VRAM_GTT, [0; 4], &mut sizes) && sizes.vram_size != 0).then_some(sizes.vram_size)
}
