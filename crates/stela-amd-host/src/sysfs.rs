//! What amdgpu and the PCI core publish about a GPU in sysfs.
use crate::chip::GcVersion;
use std::{fs, path::Path};
use stela_driver_gpu::metal::{open::PcieLink, vulkan::physical::PciBusInfo};

const DEVICES: &str = "/sys/bus/pci/devices";

/// The sysfs name of the function at `pci`, e.g. `0000:03:00.0`.
pub fn address(pci: PciBusInfo) -> String {
    format!(
        "{:04x}:{:02x}:{:02x}.{:x}",
        pci.domain, pci.bus, pci.device, pci.function
    )
}

fn read(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|text| text.trim().to_owned())
}

/// The graphics core amdgpu read from the GPU's IP discovery table: die 0,
/// the `GC` block (hardware ID 11), instance 0.
pub fn graphics_core(address: &str) -> Option<GcVersion> {
    let die = Path::new(DEVICES).join(address).join("ip_discovery/die/0");
    ["GC", "11"].iter().find_map(|block| {
        let instance = die.join(block).join("0");
        let field = |name| read(&instance.join(name))?.parse::<u8>().ok();
        Some(GcVersion::new(
            field("major")?,
            field("minor")?,
            field("revision")?,
        ))
    })
}

/// The negotiated link, from `current_link_speed` ("32.0 GT/s PCIe") and
/// `current_link_width`; zero when the kernel does not say.
pub fn link(address: &str) -> PcieLink {
    let device = Path::new(DEVICES).join(address);
    let speed = read(&device.join("current_link_speed"))
        .and_then(|text| text.split_whitespace().next().map(speed_code))
        .unwrap_or(0);
    let width = read(&device.join("current_link_width"))
        .and_then(|text| text.parse().ok())
        .unwrap_or(0);
    PcieLink { speed, width }
}

/// The Link Status speed code for a rate in GT/s.
fn speed_code(rate: &str) -> u8 {
    match rate {
        "2.5" => 1,
        "5.0" | "5" => 2,
        "8.0" | "8" => 3,
        "16.0" | "16" => 4,
        "32.0" | "32" => 5,
        "64.0" | "64" => 6,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_rates_map_to_link_status_codes() {
        assert_eq!(speed_code("2.5"), 1);
        assert_eq!(speed_code("16.0"), 4);
        assert_eq!(speed_code("32.0"), 5);
        assert_eq!(speed_code("Unknown"), 0);
    }

    #[test]
    fn a_missing_device_reads_as_unknown() {
        assert_eq!(graphics_core("ffff:ff:1f.7"), None);
        assert_eq!(link("ffff:ff:1f.7"), PcieLink { speed: 0, width: 0 });
    }
}
