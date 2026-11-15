//! What each Vulkan device a driver offers would be to the native backend,
//! without creating a device: the snapshot [`NativeBackend::new`] takes,
//! the requirements it refuses a device for, and whether it imports
//! dma-bufs. Hosted tools use it to qualify a GPU and its driver on a
//! development machine before a boot image carries them.
//!
//! [`NativeBackend::new`]: super::NativeBackend::new
use super::{Extensions, Loader, failure, physical, vk};
use crate::metal::{
    error::Result,
    vulkan::{
        backend::{Requirement, requirements},
        physical::PhysicalDevice,
    },
};
use alloc::{boxed::Box, string::String, vec::Vec};

/// One Vulkan device as the backend would see it.
#[derive(Clone, Debug)]
pub struct Surveyed {
    /// The driver's index of the device.
    pub index: usize,
    /// `VkPhysicalDeviceProperties::deviceName`.
    pub name: String,
    pub vendor_id: u32,
    pub device_id: u32,
    /// What the backend found.
    pub outcome: Outcome,
}

#[derive(Clone, Debug)]
pub enum Outcome {
    /// The device's driver is not one Stela admits, or its properties
    /// could not be read.
    NotAdmitted,
    Probed(Box<Probed>),
}

#[derive(Clone, Debug)]
pub struct Probed {
    pub snapshot: PhysicalDevice,
    /// The features the backend refuses a device without.
    pub requirements: [Requirement; 19],
}

impl Probed {
    /// The backend would create its device: no requirement is missing and
    /// the driver imports dma-bufs.
    pub fn usable(&self) -> bool {
        self.missing().next().is_none() && self.snapshot.limits.dma_buf_import
    }

    /// The names of the missing requirements.
    pub fn missing(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.requirements
            .iter()
            .filter(|requirement| !requirement.present)
            .map(|requirement| requirement.name)
    }
}

/// Surveys every device the driver behind `loader` offers.
///
/// # Safety
///
/// `loader.get_instance_proc_addr` is a Vulkan driver's (or loader's) entry
/// point.
pub unsafe fn survey(loader: &Loader) -> Result<Vec<Surveyed>> {
    let static_fn = ash::StaticFn {
        get_instance_proc_addr: loader.get_instance_proc_addr,
    };
    // SAFETY: the caller guarantees the entry point.
    let entry = unsafe { ash::Entry::from_static_fn(static_fn) };
    let application = vk::ApplicationInfo::default()
        .application_name(c"Stela Metal survey")
        .engine_name(c"Stela")
        .api_version(vk::API_VERSION_1_3);
    let info = vk::InstanceCreateInfo::default().application_info(&application);
    // SAFETY: the create info and its application info outlive the call.
    let instance = unsafe { entry.create_instance(&info, None) }.map_err(failure)?;
    // SAFETY: the instance is valid.
    let devices = unsafe { instance.enumerate_physical_devices() };
    let surveyed = devices.map_err(failure).map(|devices| {
        devices
            .iter()
            .enumerate()
            .map(|(index, &physical)| {
                // SAFETY: `physical` came from this instance.
                unsafe { one(&instance, index, physical, loader.log) }
            })
            .collect()
    });
    // SAFETY: nothing else uses the instance, and no object of it remains.
    unsafe { instance.destroy_instance(None) };
    surveyed
}

/// # Safety
///
/// `physical` came from `instance`.
unsafe fn one(
    instance: &ash::Instance,
    index: usize,
    physical: vk::PhysicalDevice,
    log: fn(&str),
) -> Surveyed {
    // SAFETY: the caller guarantees `physical`.
    let properties = unsafe { instance.get_physical_device_properties(physical) };
    let name = properties
        .device_name_as_c_str()
        .map(|name| String::from(name.to_string_lossy()))
        .unwrap_or_default();
    // SAFETY: the caller guarantees `physical`.
    let outcome = match unsafe { instance.enumerate_device_extension_properties(physical) } {
        Ok(available) => {
            let extensions = Extensions::of(&available);
            match physical::probe(instance, physical, &extensions) {
                Some(snapshot) => Outcome::Probed(Box::new(Probed {
                    requirements: requirements(&snapshot.features),
                    snapshot,
                })),
                None => Outcome::NotAdmitted,
            }
        }
        Err(error) => {
            log(&alloc::format!(
                "Vulkan device {index}: extensions not enumerated: {error:?}"
            ));
            Outcome::NotAdmitted
        }
    };
    Surveyed {
        index,
        name,
        vendor_id: properties.vendor_id,
        device_id: properties.device_id,
        outcome,
    }
}
