//! Opens the Metal service for one AMD GPU: admits its Vulkan device,
//! derives the platform facts Vulkan does not report, and serves a
//! [`Device`] over the backend and compiler. [`open`] composes the native
//! stack as the NVIDIA route does: RADV through [`NativeBackend`] and an
//! [`AirCompiler`] for the device RADV reports.
use super::{Attachment, Chip, GcVersion, Refusal};
#[cfg(feature = "metal-vulkan")]
use crate::metal::{
    compiler::{self, AirCompiler},
    native::{Loader, MemoryExporter, NativeBackend, vk},
};
use crate::metal::{
    device::Device,
    error::Error,
    service::{Service, identity::DeviceEpoch},
    shader::Compiler,
    vulkan::{backend::Backend, physical::PciBusInfo},
};
#[cfg(feature = "metal-vulkan")]
use alloc::boxed::Box;

/// The GPU the platform hands the Metal device.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Gpu {
    pub pci: PciBusInfo,
    /// The graphics core amdgpu's IP discovery reports.
    pub gc: GcVersion,
    pub attachment: Attachment,
}

/// Why the Metal service did not open.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenError {
    /// The device is not an AMD GPU this route drives.
    Refused(Refusal),
    /// The device lacks a requirement or could not be created.
    Device(Error),
}

impl From<Error> for OpenError {
    fn from(error: Error) -> Self {
        Self::Device(error)
    }
}

/// Serves the Metal device over `backend`, which drives `gpu`, and
/// `compiler`, in the gate's `epoch` for it.
pub fn serve<B: Backend, C: Compiler>(
    backend: B,
    compiler: C,
    gpu: &Gpu,
    epoch: DeviceEpoch,
) -> Result<Service<B, C>, OpenError> {
    Ok(Service::new(device(backend, compiler, gpu)?, epoch))
}

/// The Metal device over `backend`, once it is admitted as `gpu`.
fn device<B: Backend, C: Compiler>(
    backend: B,
    compiler: C,
    gpu: &Gpu,
) -> Result<Device<B, C>, OpenError> {
    let identity = &backend.physical_device().identity;
    if identity.pci_bus != Some(gpu.pci) {
        return Err(Error::Invalid("the backend drives another GPU").into());
    }
    let chip = Chip::admit(identity, gpu.gc).map_err(OpenError::Refused)?;
    let platform = chip.platform(gpu.attachment);
    Ok(Device::new(backend, compiler, &platform)?)
}

/// The Metal service over RADV and the AIR compiler.
#[cfg(feature = "metal-vulkan")]
pub type NativeService = Service<NativeBackend, AirCompiler>;

/// Opens RADV's device for `gpu` and serves it with the AIR compiler service
/// `compiler` answers for that device. The native stack is the vendor-neutral
/// one the NVIDIA route builds ([`crate::metal::nvidia::open::stack`]).
///
/// # Safety
///
/// `loader` is RADV's entry point and `compiler` answers the AIR compiler
/// service with RADV's NIR options for the device it is given, both
/// outliving the service.
#[cfg(feature = "metal-vulkan")]
pub unsafe fn open(
    loader: Loader,
    exporter: Box<dyn MemoryExporter>,
    compiler: impl FnOnce(vk::PhysicalDevice) -> compiler::Service,
    gpu: &Gpu,
    epoch: DeviceEpoch,
) -> Result<NativeService, OpenError> {
    // SAFETY: the caller's guarantees.
    let (backend, compiler) =
        unsafe { crate::metal::nvidia::open::stack(loader, gpu.pci, exporter, compiler) }?;
    serve(backend, compiler, gpu, epoch)
}

/// Opens RADV's device for `gpu` as [`open`] does, for a service whose device
/// was lost ([`Service::with_reopen`]).
///
/// # Safety
///
/// As for [`open`].
#[cfg(feature = "metal-vulkan")]
pub unsafe fn reopen(
    loader: Loader,
    exporter: Box<dyn MemoryExporter>,
    compiler: impl FnOnce(vk::PhysicalDevice) -> compiler::Service,
    gpu: &Gpu,
) -> Result<Device<NativeBackend, AirCompiler>, Error> {
    // SAFETY: the caller's guarantees.
    let (backend, compiler) =
        unsafe { crate::metal::nvidia::open::stack(loader, gpu.pci, exporter, compiler) }?;
    device(backend, compiler, gpu).map_err(|error| match error {
        OpenError::Device(error) => error,
        OpenError::Refused(_) => Error::Invalid("the GPU was refused as it reopened"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metal::{
        amd::{CHIPS, PcieLink, VENDOR_ID},
        device::fixture::{MockBackend, MockCompiler},
        vulkan::DriverId,
    };
    use core::num::NonZeroU64;

    fn gpu(gc: GcVersion) -> Gpu {
        Gpu {
            pci: PciBusInfo {
                domain: 0,
                bus: 1,
                device: 0,
                function: 0,
            },
            gc,
            attachment: Attachment {
                link: PcieLink {
                    speed: 5,
                    width: 16,
                },
                external: false,
                location_number: 0,
            },
        }
    }

    /// The mock backend presenting itself as RADV on a Radeon.
    fn radv() -> MockBackend {
        let mut backend = MockBackend::new();
        let identity = &mut backend.physical.as_mut().unwrap().identity;
        identity.vendor_id = VENDOR_ID;
        identity.driver_id = DriverId::MesaRadv;
        backend
    }

    fn serve_on(
        backend: MockBackend,
        gpu: &Gpu,
    ) -> Result<Service<MockBackend, MockCompiler>, OpenError> {
        let epoch = DeviceEpoch::from_registered(NonZeroU64::MIN);
        serve(backend, MockCompiler::default(), gpu, epoch)
    }

    #[test]
    fn an_admitted_navi48_is_served_with_its_platform() {
        let navi48 = CHIPS[1];
        let mut service = serve_on(radv(), &gpu(navi48.gc)).unwrap();
        let capabilities = service.device().capabilities();
        assert_eq!(capabilities.architecture, "GFX1201");
        assert!(!capabilities.removable);
        assert_eq!(capabilities.location_number, 0);
    }

    #[test]
    fn other_gpus_drivers_and_cores_are_refused() {
        let navi48 = CHIPS[1].gc;
        let mut other = gpu(navi48);
        other.pci.bus = 2;
        assert_eq!(
            serve_on(radv(), &other).err(),
            Some(OpenError::Device(Error::Invalid(
                "the backend drives another GPU"
            )))
        );
        let navi31 = GcVersion::new(11, 0, 0);
        assert_eq!(
            serve_on(radv(), &gpu(navi31)).err(),
            Some(OpenError::Refused(Refusal::PreRdna4 { gc: navi31 }))
        );
        // NVK on an NVIDIA board is the other route's device.
        assert_eq!(
            serve_on(MockBackend::new(), &gpu(navi48)).err(),
            Some(OpenError::Refused(Refusal::NotAmd {
                vendor_id: crate::metal::nvidia::VENDOR_ID
            }))
        );
        // A Radeon behind another Vulkan driver (here Venus) is not RADV.
        let mut venus = radv();
        venus.physical.as_mut().unwrap().identity.driver_id = DriverId::MesaVenus;
        assert_eq!(
            serve_on(venus, &gpu(navi48)).err(),
            Some(OpenError::Refused(Refusal::NotRadv(DriverId::MesaVenus)))
        );
    }
}
