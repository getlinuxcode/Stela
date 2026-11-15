//! Discrete AMD Radeon identity shared by boot and the RADV device path.
//!
//! amdgpu does not name these parts by PCI ID: since Navi 3x it reads the
//! IP discovery table and drives each block by its IP version, and Mesa's
//! `amd_family` follows the graphics core (GC) version the kernel reports.
//! This table is therefore keyed by the GC version, as the NVIDIA table is
//! keyed by nouveau's chipset; the PCI IDs are kept only so the boot screen
//! can name a card before any driver has run. The versions follow the
//! kernel's `Documentation/gpu/amdgpu/dgpu-asic-info-table.csv`.

/// The GPU generation, which fixes RADV's command and shader paths.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum Architecture {
    Rdna4,
}

/// A graphics core IP version as amdgpu's IP discovery reports it
/// (`AMDGPU_INFO_HW_IP_INFO` for `AMDGPU_HW_IP_GFX`, or the GC entry of the
/// discovery table).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct GcVersion {
    pub major: u8,
    pub minor: u8,
    pub revision: u8,
}

impl GcVersion {
    pub const fn new(major: u8, minor: u8, revision: u8) -> Self {
        Self {
            major,
            minor,
            revision,
        }
    }
}

impl core::fmt::Display for GcVersion {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.revision)
    }
}

/// One graphics core from amdgpu's discovery tables.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Chip {
    pub gc: GcVersion,
    /// Mesa's family name (`ac_get_family_name`), as RADV reports it in the
    /// device name, e.g. "AMD Radeon RX 9070 XT (RADV GFX1201)".
    pub name: &'static str,
    /// The die name people and `lspci` use.
    pub die: &'static str,
    /// The products built on this die, as the boot screen names them.
    pub products: &'static str,
    /// PCI device IDs of the die's discrete boards.
    pub pci_devices: &'static [u16],
    pub architecture: Architecture,
    /// amdgpu instantiates a display core (DCN) for this part.
    pub display: bool,
}

/// The RDNA 4 discrete parts: Navi 48 (GC 12.0.1, DCN 4.0.1) and Navi 44
/// (GC 12.0.0, DCN 4.0.1).
pub static CHIPS: [Chip; 2] = [
    Chip {
        gc: GcVersion::new(12, 0, 0),
        name: "GFX1200",
        die: "Navi 44",
        products: "Radeon RX 9060 series",
        pci_devices: &[0x7590],
        architecture: Architecture::Rdna4,
        display: true,
    },
    Chip {
        gc: GcVersion::new(12, 0, 1),
        name: "GFX1201",
        die: "Navi 48",
        products: "Radeon RX 9070 series",
        pci_devices: &[0x7550],
        architecture: Architecture::Rdna4,
        display: true,
    },
];

/// The first graphics core Stela's AMD route admits.
pub const FIRST_SUPPORTED_GC: GcVersion = GcVersion::new(12, 0, 0);

impl Chip {
    /// The chip whose graphics core is exactly `gc`.
    pub fn find(gc: GcVersion) -> Option<&'static Chip> {
        CHIPS.iter().find(|chip| chip.gc == gc)
    }

    /// The chip a discrete board's PCI device ID belongs to, for naming it
    /// before amdgpu has read its discovery table.
    pub fn from_pci_device(device: u16) -> Option<&'static Chip> {
        CHIPS.iter().find(|chip| chip.pci_devices.contains(&device))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn the_table_is_sorted_unique_and_rdna4() {
        for pair in CHIPS.windows(2) {
            assert!(pair[0].gc < pair[1].gc, "{pair:?}");
        }
        for chip in &CHIPS {
            assert!(chip.gc >= FIRST_SUPPORTED_GC);
            assert_eq!(chip.architecture, Architecture::Rdna4);
            assert!(chip.name.starts_with("GFX12"));
            assert!(!chip.pci_devices.is_empty());
        }
    }

    #[test]
    fn navi48_is_found_by_core_and_by_pci_id() {
        let chip = Chip::find(GcVersion::new(12, 0, 1)).unwrap();
        assert_eq!(chip.name, "GFX1201");
        assert_eq!(chip.die, "Navi 48");
        assert_eq!(Chip::from_pci_device(0x7550), Some(chip));
        assert_eq!(
            Chip::from_pci_device(0x7590).map(|chip| chip.die),
            Some("Navi 44")
        );
        assert_eq!(Chip::from_pci_device(0x744c), None);
        assert_eq!(Chip::find(GcVersion::new(11, 0, 0)), None);
        assert_eq!(GcVersion::new(12, 0, 1).to_string(), "12.0.1");
    }
}
