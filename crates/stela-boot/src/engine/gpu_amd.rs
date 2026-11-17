//! AMD fork (J6): claiming an RDNA 4 Radeon for amdgpu.
//!
//! As [`super::claim`] does for an NVIDIA GPU, with what differs on a
//! Radeon: its register window is BAR 5 (BAR 0 is VRAM and BAR 2 the
//! doorbells), it has no PMC_BOOT_0, and its firmware is amdgpu's pinned
//! files (`stela_loader::host_firmware::recipes::AMDGPU`), found in the
//! USB's bundle by digest and bound to the names amdgpu requests.
use super::*;

const AMD: u16 = 0x1002;
/// The register BAR of an RDNA GPU (amdgpu's `rmmio`, BAR 5 since Bonaire).
const REGISTER_BAR: usize = 5;
/// The DMA addresses an RDNA 4 GPU reaches: GMC v12 sets a 44-bit mask
/// (`gmc_v12_0_sw_init`), which the device refuses below its placement's
/// limit (the second trial: "No suitable DMA available").
const DMA_LIMIT_RDNA4: u64 = (1 << 44) - 1;
/// The Revision ID register in configuration space.
const REVISION_ID: u16 = 0x08;

/// amdgpu's catalogue for the Radeon with PCI device ID `device`: each of
/// its pinned files the boot's catalogue holds, by exact binding or by the
/// bundle's digest, checked against its pin. Unlike nouveau, amdgpu names
/// the file it lacks and stops there, so a partial catalogue is kept, with
/// a line naming what is missing.
fn firmware(device: u16, catalog: Option<&Arc<Catalog>>) -> Result<PciFirmware, ServiceError> {
    use stela_loader::host_firmware::{
        bundle,
        recipes::{self, AMDGPU_TARGET, Plan},
        requirements::Inventory,
    };
    let target = || {
        FirmwareTarget::new(
            AMDGPU_TARGET.driver.to_owned(),
            AMDGPU_TARGET.hardware.to_owned(),
            AMDGPU_TARGET.revision.to_owned(),
            None,
        )
        .map_err(|_| ServiceError::Refused("the GPU firmware target is invalid"))
    };
    let empty = |why: &str| {
        output::arguments(format_args!(
            "GPU 1002:{device:04x}: no amdgpu firmware: {why}\n"
        ));
        let catalog = Catalog::rebind_all(core::iter::empty())
            .map_err(|_| ServiceError::Refused("the GPU firmware catalogue cannot be made"))?;
        PciFirmware::new(target()?, Arc::new(catalog), Box::new([]))
            .map_err(|_| ServiceError::Refused("the GPU firmware image names are invalid"))
    };
    let Some(catalog) = catalog else {
        return empty("the boot has no firmware catalogue (no USB bundle)");
    };
    let mut inventory = Inventory::default();
    if recipes::require_amdgpu(&mut inventory, device).is_err() {
        return empty("no pinned files for this Radeon");
    }
    let plan = Plan::select(&inventory)
        .map_err(|_| ServiceError::Refused("the GPU firmware recipe cannot be selected"))?;
    // SAFETY: the reference count is never given back, so the catalog
    // stays allocated for every image borrowed from it for the boot.
    let stored: &'static Catalog = unsafe { &*Arc::into_raw(catalog.clone()) };
    let (mut bindings, mut names, mut missing) = (Vec::new(), Vec::new(), Vec::new());
    for selection in plan.selections() {
        let request = &selection.request;
        let found = match stored.load(request.view()) {
            Err(error) if error.kind == FirmwareError::Missing => {
                let name = bundle::name(selection.recipe.image.sha256);
                match stored.load(stela_drivers::firmware::Requirement {
                    name: &name,
                    target: bundle::TARGET,
                    minimum_bytes: request.minimum_bytes,
                }) {
                    Err(stored_error) if stored_error.kind == FirmwareError::Missing => Err(error),
                    other => other,
                }
            }
            other => other,
        };
        match found {
            Ok(image) => {
                selection
                    .recipe
                    .image
                    .verify_digest(image.bytes().len(), image.sha256())
                    .map_err(|_| {
                        ServiceError::Refused(
                            "the GPU firmware digest does not match its pinned recipe",
                        )
                    })?;
                bindings.push((image, request));
                names.push(request.name.clone());
            }
            Err(error) if error.kind == FirmwareError::Missing => {
                missing.push(request.name.clone());
            }
            Err(error) => {
                output::arguments(format_args!(
                    "GPU 1002:{device:04x}: firmware {} refused: {error:?}\n",
                    request.name
                ));
                return Err(ServiceError::Refused(
                    "the GPU firmware does not match its exact catalogue target",
                ));
            }
        }
    }
    output::arguments(format_args!(
        "GPU 1002:{device:04x}: amdgpu firmware {} of {} files{}{}\n",
        names.len(),
        names.len() + missing.len(),
        if missing.is_empty() {
            ""
        } else {
            "; the USB's bundle lacks "
        },
        missing.join(", ")
    ));
    let catalog = Catalog::rebind_all(bindings.iter().map(|(image, request)| {
        let view = request.view();
        (*image, view.name, view.target)
    }))
    .map_err(|_| ServiceError::Refused("the GPU firmware cannot be bound to its requests"))?;
    PciFirmware::new(target()?, Arc::new(catalog), names.into_boxed_slice())
        .map_err(|_| ServiceError::Refused("the GPU firmware image names are invalid"))
}

/// Claim one Radeon, as [`super::claim`] claims an NVIDIA GPU.
pub(super) fn claim(
    services: &dyn NativeServices,
    adapter: &gpu::Adapter,
    line: u32,
    gop: crate::pci_io::ScreenOwner,
    catalog: Option<&Arc<Catalog>>,
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
        dma_limit: DMA_LIMIT_RDNA4,
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
    #[cfg_attr(not(feature = "nvk"), allow(unused_variables))]
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
    let firmware = firmware(adapter.device, catalog)?;
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
    // Read again below for the DRM identity and the PCIe link.
    #[cfg_attr(not(feature = "nvk"), allow(unused_mut))]
    let mut function = ManuallyDrop::new(bound.requester.into_function());
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
    // The link the Metal device reports (`metal::amd`), as an NVIDIA
    // GPU's: its PCIe capability's Link Status.
    #[cfg(feature = "nvk")]
    let link = capabilities
        .iter()
        .find(|(_, id)| *id == PCIE_CAPABILITY)
        .map(|(offset, _)| {
            function
                .config()
                .read16(u16::from(*offset) + LINK_STATUS)
                .map(PcieLink::from_link_status)
        })
        .transpose()
        .map_err(ServiceError::Config)?;
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
        link,
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

/// Stela's own checks on each Radeon amdgpu bound (`src/amd/linux/trial.c`
/// in `stela-driver-gpu`): the GPU runs a command Stela submits, and the
/// connected outputs show colour bars through DCN for a while.
///
/// # Safety
/// Entered in the Linux owner, on the task that probed the GPUs.
pub(super) unsafe fn trial() {
    if !TRIAL.load(core::sync::atomic::Ordering::Relaxed) {
        return;
    }
    unsafe extern "C" {
        fn stela_amdgpu_trial() -> core::ffi::c_int;
    }
    // SAFETY: the caller's contract; the checks may sleep.
    let checked = unsafe { stela_amdgpu_trial() };
    output::arguments(format_args!("GPU TRIAL checked {checked} Radeon(s)\n"));
}

/// `struct stela_amdgpu_screen` (`src/amd/linux/trial.c`).
#[repr(C)]
struct Screen {
    pixels: *mut c_void,
    width: u32,
    height: u32,
    pitch: u32,
    refresh_millihertz: u32,
}

/// The screen [`trial`] lit on a Radeon's connected outputs, for Stela's
/// console: a linear XRGB8888 buffer of VRAM, mapped in the host's page
/// window, which every processor sees.
pub(in crate::engine) fn screen() -> Option<crate::console::Framebuffer> {
    unsafe extern "C" {
        fn stela_amdgpu_screen(screen: *mut Screen) -> bool;
    }
    let mut screen = Screen {
        pixels: ptr::null_mut(),
        width: 0,
        height: 0,
        pitch: 0,
        refresh_millihertz: 0,
    };
    // SAFETY: the C side only copies what its trial recorded.
    if !unsafe { stela_amdgpu_screen(&raw mut screen) } || screen.pixels.is_null() {
        return None;
    }
    Some(crate::console::Framebuffer::linear(
        screen.pixels as usize,
        screen.width as usize,
        screen.height as usize,
        screen.pitch as usize / size_of::<u32>(),
    ))
}

/// Whether this boot is a GPU trial (`--gpu-trial`), whose checks
/// [`trial`] runs once amdgpu bound the Radeon; a boot that starts macOS
/// leaves its outputs to its own display.
static TRIAL: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

pub(in crate::engine) fn set_trial() {
    TRIAL.store(true, core::sync::atomic::Ordering::Relaxed);
}
