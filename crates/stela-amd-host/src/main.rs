//! Qualifies an AMD GPU and its RADV driver for Stela's Metal device on a
//! Linux host, before any boot image carries amdgpu or RADV.
//!
//! It loads the system's Vulkan loader, surveys every device the way the
//! native backend would ([`survey`]), and for each Radeon reads the graphics
//! core and PCIe link amdgpu publishes in sysfs, admits it as the AMD Metal
//! route does ([`metal::amd`]) and derives the Metal capabilities Stela would
//! report: GPU families, and every capability missing with the layer that
//! lacks it. Shader features are taken as complete, since the AIR compiler
//! for RADV is not built here; the report is what the driver and Stela's
//! Metal layer allow.
//!
//! With `--device LIBRARY.metallib` (built with the `radv-host` feature, over
//! `libstela_radv.so`), it then opens the real Metal device on the first
//! admitted Radeon and runs its first work there ([`device`]).
//!
//! To force RADV when several drivers are installed:
//! `VK_DRIVER_FILES=/usr/share/vulkan/icd.d/radeon_icd.x86_64.json`.
//!
//! [`survey`]: stela_driver_gpu::metal::native::survey
//! [`metal::amd`]: stela_driver_gpu::metal::amd
use std::process::ExitCode;
use stela_driver_gpu::metal::{
    amd::{self, Attachment, Chip, GcVersion, open::Gpu},
    capabilities::{ApiFeatures, Capabilities, ShaderFeatures},
    native::{
        Loader,
        survey::{self, Outcome, Probed, Surveyed},
    },
    vulkan::{MemoryHeapFlags, MemoryProperties},
};

#[cfg(feature = "radv-host")]
mod device;
#[cfg(feature = "radv-host")]
mod suite;
#[cfg(feature = "radv-host")]
mod suite_blit;
#[cfg(feature = "radv-host")]
mod suite_constants;
#[cfg(feature = "radv-host")]
mod suite_render;
#[cfg(feature = "radv-host")]
mod suite_resources;
mod sysfs;

fn log(message: &str) {
    eprintln!("vulkan: {message}");
}

fn relax() {
    std::thread::yield_now();
}

fn main() -> ExitCode {
    // SAFETY: loading the system's Vulkan loader runs its initializers,
    // which is what any Vulkan application does.
    let entry = match unsafe { ash::Entry::load() } {
        Ok(entry) => entry,
        Err(error) => {
            eprintln!("cannot load the Vulkan loader (libvulkan.so.1): {error}");
            return ExitCode::FAILURE;
        }
    };
    let loader = Loader {
        get_instance_proc_addr: entry.static_fn().get_instance_proc_addr,
        log,
        submit_thread: None,
        refusals: None,
        relax,
    };
    // SAFETY: the entry point is the loaded loader's, which outlives the
    // survey.
    let surveyed = match unsafe { survey::survey(&loader) } {
        Ok(surveyed) => surveyed,
        Err(error) => {
            eprintln!(
                "Vulkan survey failed: {error:?}. With VK_DRIVER_FILES set to RADV alone, this \
                 means RADV found no AMD GPU: check that amdgpu is loaded (lsmod | grep amdgpu)"
            );
            return ExitCode::FAILURE;
        }
    };
    if surveyed.is_empty() {
        println!("No Vulkan device. Is RADV installed (mesa-vulkan-drivers)?");
        return ExitCode::FAILURE;
    }
    let work = match parse(std::env::args().skip(1).collect()) {
        Ok(work) => work,
        Err(error) => {
            eprintln!("{error}");
            eprintln!(
                "usage: stela-amd-host [--device LIBRARY.metallib | --suite DIR [--only TEXT]...]"
            );
            return ExitCode::FAILURE;
        }
    };
    let mut qualified = None;
    for device in &surveyed {
        if let Some(gpu) = report(device) {
            qualified.get_or_insert(gpu);
        }
        println!();
    }
    if let Some(gpu) = qualified {
        println!("RESULT: an AMD GPU qualifies for Stela's Metal device over RADV.");
        match work {
            None => ExitCode::SUCCESS,
            Some(work) => run_device(&gpu, work),
        }
    } else {
        println!("RESULT: no AMD GPU qualifies yet; the lines above say why.");
        ExitCode::FAILURE
    }
}

/// What the arguments ask of the Metal device: `None` to qualify only.
fn parse(args: Vec<String>) -> Result<Option<Work>, String> {
    match args.split_first() {
        None => Ok(None),
        Some((flag, [path])) if flag == "--device" => std::fs::read(path)
            .map(|bytes| Some(Work::First(bytes)))
            .map_err(|error| format!("cannot read {path}: {error}")),
        Some((flag, [dir, rest @ ..])) if flag == "--suite" => {
            let mut only = Vec::new();
            let mut rest = rest.iter();
            while let Some(arg) = rest.next() {
                match (arg.as_str(), rest.next()) {
                    ("--only", Some(text)) => only.push(text.clone()),
                    _ => return Err(format!("unexpected argument {arg}")),
                }
            }
            Ok(Some(Work::Suite(dir.into(), only)))
        }
        Some((arg, _)) => Err(format!("unexpected argument {arg}")),
    }
}

/// The work `--device` or `--suite` names.
#[cfg(feature = "radv-host")]
use device::Work;
#[cfg(not(feature = "radv-host"))]
#[expect(dead_code, reason = "only the radv-host feature runs the work")]
enum Work {
    First(Vec<u8>),
    Suite(std::path::PathBuf, Vec<String>),
}

/// Runs `--device` or `--suite` on the qualified GPU.
#[cfg(feature = "radv-host")]
fn run_device(gpu: &Gpu, work: Work) -> ExitCode {
    println!();
    println!("Metal device on {}:", sysfs::address(gpu.pci));
    match device::run(gpu, work) {
        Ok(()) => {
            println!("RESULT: Stela's Metal device ran its work on the GPU.");
            ExitCode::SUCCESS
        }
        Err(error) => {
            println!("  {error}");
            println!("RESULT: the Metal device failed on the GPU.");
            ExitCode::FAILURE
        }
    }
}

#[cfg(not(feature = "radv-host"))]
fn run_device(_: &Gpu, work: Work) -> ExitCode {
    let (Work::First(_) | Work::Suite(..)) = work;
    eprintln!("--device and --suite need the radv-host feature, which links libstela_radv.so");
    ExitCode::FAILURE
}

/// Reports one device; the GPU when it is an admitted, usable Radeon.
fn report(device: &Surveyed) -> Option<Gpu> {
    println!(
        "Vulkan device {}: {} (vendor {:04x}, device {:04x})",
        device.index, device.name, device.vendor_id, device.device_id
    );
    let probed = match &device.outcome {
        Outcome::NotAdmitted => {
            println!("  driver: not one Stela admits (RADV, NVK, ANV or Venus)");
            return None;
        }
        Outcome::Probed(probed) => probed,
    };
    let identity = &probed.snapshot.identity;
    println!(
        "  driver: {:?}, Vulkan {}.{}.{}",
        identity.driver_id,
        identity.api_version >> 22,
        (identity.api_version >> 12) & 0x3ff,
        identity.api_version & 0xfff
    );
    memory(probed);
    let usable = requirements(probed);
    if device.vendor_id != amd::VENDOR_ID {
        println!("  not an AMD GPU: the AMD route does not apply");
        return None;
    }
    let Some(pci) = identity.pci_bus else {
        println!("  the driver reports no PCI address (VK_EXT_pci_bus_info)");
        return None;
    };
    let address = sysfs::address(pci);
    println!("  PCI: {address}");
    let Some(gc) = sysfs::graphics_core(&address).or_else(|| core_from_name(&device.name)) else {
        println!("  graphics core: unknown (no ip_discovery in sysfs, no GFX name from RADV)");
        return None;
    };
    println!("  graphics core: GC {gc}");
    let attachment = Attachment {
        link: sysfs::link(&address),
        external: false,
        location_number: 0,
    };
    let gpu = Gpu {
        pci,
        gc,
        attachment,
    };
    let chip = match Chip::admit(identity, gpu.gc) {
        Ok(chip) => chip,
        Err(refusal) => {
            println!("  admission: refused, {refusal:?}");
            return None;
        }
    };
    println!(
        "  admission: {} {} ({}), PCIe gen {} x{}",
        chip.name, chip.die, chip.products, attachment.link.speed, attachment.link.width
    );
    capabilities(probed, chip, attachment);
    usable.then_some(gpu)
}

/// The device-local heap and how much of it the CPU reaches (Resizable BAR).
fn memory(probed: &Probed) {
    let memory = &probed.snapshot.memory;
    let local = memory
        .heaps
        .iter()
        .filter(|heap| heap.flags.contains(MemoryHeapFlags::DEVICE_LOCAL))
        .map(|heap| heap.size)
        .max()
        .unwrap_or(0);
    let reached = memory
        .types
        .iter()
        .filter(|kind| {
            kind.properties
                .contains(MemoryProperties::DEVICE_LOCAL | MemoryProperties::HOST_VISIBLE)
        })
        .filter_map(|kind| memory.heaps.get(kind.heap_index as usize))
        .map(|heap| heap.size)
        .max()
        .unwrap_or(0);
    println!(
        "  memory: {} MiB device-local, {} MiB reached by the CPU{}",
        local >> 20,
        reached >> 20,
        if local > 0 && reached < local - local / 8 {
            " (enable Resizable BAR in the firmware)"
        } else {
            ""
        }
    );
}

/// Prints the backend's requirements; true when none is missing.
fn requirements(probed: &Probed) -> bool {
    let missing: Vec<_> = probed.missing().collect();
    if missing.is_empty() {
        println!(
            "  backend requirements: all {} present",
            probed.requirements.len()
        );
    } else {
        println!("  backend requirements missing: {}", missing.join(", "));
    }
    if !probed
        .snapshot
        .features
        .dynamic_primitive_topology_unrestricted
    {
        println!(
            "  primitive topology: restricted to the pipeline's class; \
             point and line draws of unspecified pipelines compile a variant, \
             and GPU-encoded commands cannot switch to those pipelines"
        );
    }
    println!(
        "  dma-buf import: {}",
        if probed.snapshot.limits.dma_buf_import {
            "yes"
        } else {
            "NO (the backend refuses the device)"
        }
    );
    probed.usable()
}

/// The Metal profile Stela would derive, with every gap and its cause.
fn capabilities(probed: &Probed, chip: &Chip, attachment: Attachment) {
    let capabilities = Capabilities::derive(
        &probed.snapshot,
        &chip.platform(attachment),
        ShaderFeatures::KNOWN,
        ApiFeatures::IMPLEMENTED,
    );
    println!("  Metal architecture: {}", capabilities.architecture);
    println!("  Metal GPU families: {:?}", capabilities.families());
    let gaps = capabilities.gaps();
    if gaps.is_empty() {
        println!("  Metal gaps: none");
    } else {
        println!("  Metal gaps ({}):", gaps.len());
        for gap in gaps {
            println!("    - {} ({:?})", gap.capability, gap.cause);
        }
    }
}

/// The graphics core RADV names a device by, e.g. "(RADV GFX1201)", for
/// hosts whose kernel publishes no IP discovery table.
fn core_from_name(name: &str) -> Option<GcVersion> {
    amd::CHIPS
        .iter()
        .find(|chip| name.contains(&format!("RADV {}", chip.name)))
        .map(|chip| chip.gc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use stela_driver_gpu::metal::vulkan::physical::PciBusInfo;

    #[test]
    fn radv_names_give_the_core() {
        assert_eq!(
            core_from_name("AMD Radeon RX 9070 XT (RADV GFX1201)"),
            Some(GcVersion::new(12, 0, 1))
        );
        assert_eq!(
            core_from_name("AMD Radeon RX 9060 XT (RADV GFX1200)"),
            Some(GcVersion::new(12, 0, 0))
        );
        assert_eq!(core_from_name("AMD Radeon RX 7900 XTX (RADV NAVI31)"), None);
    }

    #[test]
    fn addresses_are_sysfs_names() {
        let pci = PciBusInfo {
            domain: 0,
            bus: 3,
            device: 0,
            function: 0,
        };
        assert_eq!(sysfs::address(pci), "0000:03:00.0");
    }
}
