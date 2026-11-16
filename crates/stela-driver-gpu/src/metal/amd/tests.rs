use super::*;
use crate::metal::{abi::DeviceLocation, nvidia::fixture::nvk};

const GEN5_X16: PcieLink = PcieLink {
    speed: 5,
    width: 16,
};

fn navi48() -> &'static Chip {
    Chip::find(GcVersion::new(12, 0, 1)).expect("Navi 48 is in the table")
}

/// A RADV identity on a discrete Radeon, built from the NVK snapshot with
/// only the fields admission reads changed.
fn radv_identity() -> Identity {
    let mut identity = nvk(&crate::metal::nvidia::CHIPS[0], 16 << 30).identity;
    identity.vendor_id = VENDOR_ID;
    identity.device_id = 0x7550;
    identity.driver_id = DriverId::MesaRadv;
    identity.device_name = String::from("AMD Radeon RX 9070 XT (RADV GFX1201)");
    identity
}

#[test]
fn admission_requires_a_radv_discrete_rdna4_core() {
    let identity = radv_identity();
    assert_eq!(Chip::admit(&identity, navi48().gc), Ok(navi48()));
    assert_eq!(
        Chip::admit(&identity, GcVersion::new(12, 0, 0)).map(|chip| chip.die),
        Ok("Navi 44")
    );

    let rdna3 = GcVersion::new(11, 0, 0);
    assert_eq!(
        Chip::admit(&identity, rdna3),
        Err(Refusal::PreRdna4 { gc: rdna3 })
    );
    let unknown = GcVersion::new(12, 9, 0);
    assert_eq!(
        Chip::admit(&identity, unknown),
        Err(Refusal::UnknownCore { gc: unknown })
    );

    let mut apu = radv_identity();
    apu.device_type = PhysicalDeviceType::IntegratedGpu;
    assert_eq!(
        Chip::admit(&apu, navi48().gc),
        Err(Refusal::NotDiscrete(PhysicalDeviceType::IntegratedGpu))
    );

    let mut nvidia = radv_identity();
    nvidia.vendor_id = 0x10de;
    assert_eq!(
        Chip::admit(&nvidia, navi48().gc),
        Err(Refusal::NotAmd { vendor_id: 0x10de })
    );

    let mut nvk_on_radeon = radv_identity();
    nvk_on_radeon.driver_id = DriverId::MesaNvk;
    assert_eq!(
        Chip::admit(&nvk_on_radeon, navi48().gc),
        Err(Refusal::NotRadv(DriverId::MesaNvk))
    );
}

#[test]
fn platform_reports_placement_bandwidth_and_alignments() {
    let slot = navi48().platform(Attachment {
        link: GEN5_X16,
        external: false,
        location_number: 0,
    });
    assert_eq!(slot.architecture, "GFX1201");
    assert_eq!(slot.location, DeviceLocation::Slot);
    assert!(!slot.headless);
    assert!(!slot.removable);
    // PCIe 5.0 x16: 32 GT/s with 128b/130b encoding, about 63 GB/s.
    assert_eq!(
        slot.max_transfer_rate,
        32000 * 128 / 130 * 16 * 1_000_000 / 8
    );
    assert_eq!(slot.linear_row_alignment, 128);
    assert_eq!(slot.linear_offset_alignment, 256);
    assert_eq!(slot.sparse_page_bytes, 64 << 10);

    let egpu = navi48().platform(Attachment {
        link: PcieLink { speed: 3, width: 4 },
        external: true,
        location_number: 1,
    });
    assert_eq!(egpu.location, DeviceLocation::External);
    assert!(egpu.removable);
    assert_eq!(egpu.location_number, 1);
}

#[test]
fn alignments_are_what_rdna4_linear_surfaces_need() {
    // addrlib's linear pitch on GFX12, and RADV's linear memory alignment,
    // both measured on the RX 9070 XT.
    assert_eq!(LINEAR_ROW_ALIGNMENT % 128, 0);
    assert_eq!(LINEAR_OFFSET_ALIGNMENT % 256, 0);
}

/// The derivation is vendor-neutral: over a driver that reports what NVK
/// reports, the Navi 48 platform reaches the same Metal 3 and Apple7
/// families. RADV's own snapshot is what `stela-amd-host` checks on
/// hardware.
#[test]
fn an_nvk_like_snapshot_on_navi48_reaches_metal3_and_apple7() {
    use crate::metal::{
        abi::GpuFamily,
        capabilities::{ApiFeatures, Capabilities, ShaderFeatures},
    };
    let mut snapshot = nvk(&crate::metal::nvidia::CHIPS[0], 16 << 30);
    snapshot.identity = radv_identity();
    let capabilities = Capabilities::derive(
        &snapshot,
        &navi48().platform(Attachment {
            link: GEN5_X16,
            external: false,
            location_number: 0,
        }),
        ShaderFeatures::KNOWN,
        ApiFeatures::KNOWN,
    );
    assert_eq!(capabilities.architecture, "GFX1201");
    for family in [
        GpuFamily::Mac2,
        GpuFamily::Common3,
        GpuFamily::Apple7,
        GpuFamily::Metal3,
    ] {
        assert!(capabilities.supports_family(family), "{family:?}");
    }
}

#[test]
fn radv_runs_apples_simd_width() {
    use crate::metal::capabilities::simd_width;
    // RADV compiles compute at 32 or 64 lanes and defaults to 64: Metal code
    // for Apple GPUs takes 32-wide SIMD groups.
    assert_eq!(simd_width(32, 64), 32);
    // NVK and ANV.
    assert_eq!(simd_width(32, 32), 32);
    assert_eq!(simd_width(8, 32), 32);
    // A driver that cannot run 32 keeps its widest.
    assert_eq!(simd_width(64, 64), 64);
}
