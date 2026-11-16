//! AMD fork (J6): claiming an RDNA 4 Radeon for amdgpu.
//!
//! As [`super::claim`] does for an NVIDIA GPU, with what differs on a
//! Radeon: its register window is BAR 5 (BAR 0 is VRAM and BAR 2 the
//! doorbells), it has no PMC_BOOT_0, and the firmware amdgpu requests is not
//! yet pinned, so its catalogue is empty and amdgpu's requests fail as
//! missing files (`request_firmware` returns -ENOENT) until it is.
use super::*;

const AMD: u16 = 0x1002;
/// The register BAR of an RDNA GPU (amdgpu's `rmmio`, BAR 5 since Bonaire).
const REGISTER_BAR: usize = 5;
/// The Revision ID register in configuration space.
const REVISION_ID: u16 = 0x08;

/// The empty catalogue an AMD GPU starts with, under amdgpu's target.
fn firmware(device: u16, revision: u8) -> Result<PciFirmware, ServiceError> {
    let target = FirmwareTarget::new(
        "amdgpu".to_owned(),
        format!("1002:{device:04x}"),
        format!("{revision:02x}"),
        None,
    )
    .map_err(|_| ServiceError::Refused("the GPU firmware target is invalid"))?;
    let catalog = Catalog::rebind_all(core::iter::empty())
        .map_err(|_| ServiceError::Refused("the GPU firmware catalogue cannot be made"))?;
    PciFirmware::new(target, Arc::new(catalog), Box::new([]))
        .map_err(|_| ServiceError::Refused("the GPU firmware image names are invalid"))
}

/// Claim one Radeon, as [`super::claim`] claims an NVIDIA GPU.
pub(super) fn claim(
    services: &dyn NativeServices,
    adapter: &gpu::Adapter,
    line: u32,
    gop: crate::pci_io::ScreenOwner,
) -> Result<Claimed, ServiceError> {
    use crate::pci_io::ScreenOwner;
    let address = adapter.address;
    let timer = services.minimum_delay()?;
    let mut requester = services.prepare_requester(address)?;
    let bus_reset = requester.bus_reset_available();
    let function = requester.function_mut();
    if (function.vendor, function.device, function.class) != (AMD, adapter.device, adapter.class) {
        return Err(ServiceError::Refused("the GPU's original identity changed"));
    }
    let translated = services.dma_translated(address);
    gpu::cold::check_candidate(function, timer.as_ref(), translated, bus_reset)
        .map_err(|error| cold_refusal(address, error))?;
    let owns_screen = match gop {
        ScreenOwner::Function(source) if source.address == address => {
            source.bar.words().try_for_each(|(offset, expected)| {
                let observed = function
                    .config()
                    .read32(offset)
                    .map_err(ServiceError::Config)?;
                if observed != expected {
                    return Err(ServiceError::Config(ConfigError::before_effect(
                        ConfigFailure::Readback {
                            offset,
                            width: 4,
                            expected,
                            observed,
                        },
                    )));
                }
                Ok(())
            })?;
            true
        }
        ScreenOwner::Unknown(apart) => {
            stela_boot_core::firmware_screen::gpu_may_show(&apart, address.segment, address.bus)
        }
        ScreenOwner::Function(_) | ScreenOwner::NoScreen => false,
    };
    // BAR sizing below temporarily turns off memory decode.
    if owns_screen && !crate::console::retire_firmware() {
        return Err(ServiceError::Refused(
            "a firmware framebuffer writer is still active",
        ));
    }
    let mut placement = Placement {
        bus: address.bus,
        devfn: address.device << 3 | address.function,
        irq: line,
        dma_limit: DMA_LIMIT,
        ..Placement::default()
    };
    let (mut bars, mut resources) = (Vec::new(), ManuallyDrop::new(Vec::new()));
    let function_bars = function.bars().map_err(ServiceError::Config)?;
    if let Err(why) = gpu::cold::register_bar(function_bars[REGISTER_BAR], PAGE) {
        output::arguments(format_args!(
            "GPU {address} register BAR refused: {why:?} BAR5={:?}\n",
            function_bars[REGISTER_BAR]
        ));
        return Err(ServiceError::Refused(
            "the GPU's BAR 5 is not an assigned register window",
        ));
    }
    // The register window's resource, for the observation below.
    let mut registers_at = None;
    for (index, (region, bar)) in placement.regions.iter_mut().zip(function_bars).enumerate() {
        let Some(
            bar @ Bar::Memory {
                address: start,
                size,
                prefetchable,
                wide,
            },
        ) = bar
        else {
            continue;
        };
        let resource = services.bar_resource(address, bar)?;
        let physical = resource.physical_span();
        if physical.start() != start || physical.bytes() != size {
            return Err(ServiceError::Refused(
                "the GPU BAR is translated by the host bridge",
            ));
        }
        if index == REGISTER_BAR {
            registers_at = Some(resources.len());
        }
        resources.push(resource);
        bars.push(start..start + size);
        *region = Region {
            start,
            size,
            flags: RESOURCE_MEMORY
                | if prefetchable {
                    RESOURCE_PREFETCHABLE
                } else {
                    0
                }
                | if wide { RESOURCE_WIDE } else { 0 },
        };
    }
    let registers_at = registers_at.ok_or(ServiceError::Refused(
        "the GPU's register window has no host resource",
    ))?;
    let decode = gpu::cold::enable_decode(function, timer.as_ref()).map_err(|error| {
        output::arguments(format_args!(
            "GPU {address} memory decoding refused: {error:?}\n"
        ));
        ServiceError::Refused("the GPU's memory decoding did not turn on")
    })?;
    if decode.power_state != 0 || decode.memory_was_off {
        output::arguments(format_args!(
            "GPU {address}: firmware left it in D{} with memory decoding {}; brought to D0 with decoding on\n",
            decode.power_state,
            if decode.memory_was_off { "off" } else { "on" }
        ));
    }
    let config = function
        .config()
        .retain_epoch()
        .map_err(ServiceError::Mapping)?;
    let pci_words = gpu_pci_words(function).map_err(ServiceError::Config)?;
    let revision = function
        .config()
        .read8(REVISION_ID)
        .map_err(ServiceError::Config)?;
    let registers = services
        .authority()?
        .map(
            resources[registers_at].clone(),
            0,
            PAGE as usize,
            CacheAttribute::Uncached,
            Protection::ReadOnly,
        )
        .map_err(ServiceError::Mapping)?;
    // MM_INDEX, the first register: a harmless read the views compare.
    let index = registers.read32(0).map_err(ServiceError::Mapping)?;
    if index == u32::MAX {
        return Err(ServiceError::Refused("the GPU's registers do not answer"));
    }
    let firmware = firmware(adapter.device, revision)?;
    if let Some(chip) = adapter.amd_chip() {
        crate::host_inventory::text(alloc::format!("GPU{line}Chip"), chip.name);
    }
    crate::host_inventory::text(
        alloc::format!("GPU{line}PCIID"),
        alloc::format!("1002:{:04x}", adapter.device),
    );
    let mut cold = gpu::cold::Cold::new(gpu::cold::Inputs {
        requester,
        views: GpuViews {
            config,
            pci_words,
            registers,
            boot0: index,
        },
        timer,
    });
    if translated {
        // SAFETY: the mount's active remapper translates this requester, and
        // its private domain is the one the cold attempt binds.
        cold = unsafe { cold.translated() };
    }
    let ready = cold
        .quiesce_and_bind()
        .map_err(|failure| cold_refusal(address, failure.error))?;
    let sizes: Vec<(usize, u64)> = placement
        .regions
        .iter()
        .enumerate()
        .filter(|(_, region)| region.size != 0)
        .map(|(index, region)| (index, region.size))
        .collect();
    let sizes = stela_boot_core::gpu_status::bar_sizes(&sizes);
    let reset = ready.reset().is_some();
    output::arguments(format_args!(
        "GPU {address}: Radeon claimed for amdgpu, reset {:?}; {sizes}\n",
        ready.reset()
    ));
    // SAFETY: both original owners transfer into this retained function
    // lifetime before Linux probe. Only the temporary UC observation is retired.
    let (bound, views) = unsafe { ready.into_parts() };
    let attachment = ManuallyDrop::new(bound.attachment);
    let function = ManuallyDrop::new(bound.requester.into_function());
    if reset {
        resized_bars(address, &function, &mut placement, &mut bars);
    }
    let mut views = ManuallyDrop::new(views);
    // A reset may change MM_INDEX: the observation is checked before it.
    // SAFETY: this scoped BAR observation was the sole user of the UC mapping.
    unsafe { views.registers.retire() }.map_err(ServiceError::Mapping)?;
    let capabilities = function
        .capabilities()
        .map_err(ServiceError::PciCapability)?;
    let msi = capabilities
        .iter()
        .find(|(_, id)| *id == MSI_CAPABILITY)
        .map(|(offset, _)| Msi::at(u16::from(*offset)));
    #[cfg(feature = "nvk")]
    let drm = {
        let config = function.config();
        let subsystem = config.read32(SUBSYSTEM_ID).map_err(ServiceError::Config)?;
        client::DrmDevice {
            segment: address.segment,
            vendor: function.vendor,
            device: function.device,
            subsystem_vendor: subsystem as u16,
            subsystem_device: (subsystem >> 16) as u16,
            bus: address.bus,
            slot: address.device,
            function: address.function,
            revision,
        }
    };
    Ok(Claimed {
        shows_screen: owns_screen,
        device: Device {
            config: SharedConfig(Rc::new(RefCell::new(ManuallyDrop::into_inner(function)))),
            msi,
            custody: FunctionCustody::new(ManuallyDrop::into_inner(attachment)),
            _views: views,
            _resources: resources,
        },
        firmware,
        placement,
        bars,
        name: format!("{address}"),
        #[cfg(feature = "nvk")]
        drm,
        #[cfg(feature = "nvk")]
        link: None,
        amd: true,
    })
}

/// Start the Linux image of the claimed GPUs' driver: amdgpu's when they
/// are Radeons, else nouveau's.
///
/// # Safety
/// As [`gpu::nouveau::start`].
pub(super) unsafe fn start(claimed: &[Claimed]) -> Result<(), Errno> {
    if claimed.iter().any(|claimed| claimed.amd) {
        // SAFETY: the caller's contract.
        unsafe { gpu::amdgpu::start() }
    } else {
        // SAFETY: the caller's contract.
        unsafe { gpu::nouveau::start() }
    }
}
