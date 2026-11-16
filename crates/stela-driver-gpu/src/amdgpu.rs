//! AMD fork (J6): amdgpu in a Linux environment, the initcalls that register
//! it, DRM and the subsystems beneath them (`build/amdgpu.rs`).
use core::{
    ffi::{c_int, c_void},
    ptr,
};
use stela_linux::owner::Errno;

unsafe extern "C" {
    static stela_linux_amdgpu_image: c_void;
    /// amdgpu's `runpm` parameter (`amdgpu_drv.c`).
    static mut amdgpu_runtime_pm: c_int;
    fn stela_linux_start(image: *const c_void, registry: *mut *mut c_void) -> c_int;
}

/// Bring Linux up with amdgpu as far as a device can bind, stopping at the
/// first stage that fails. Each environment starts one image, this one or
/// nouveau's, once.
///
/// # Safety
/// As [`crate::nouveau::start`]: on the canonical Linux owner's original
/// managed task after Initial and SRCU startup have completed.
pub unsafe fn start() -> Result<(), Errno> {
    // Stela keeps the GPU powered: no runtime suspend (BACO, BOCO), whose
    // wake-up path (`pci_wake_from_d3`) the environment does not serve.
    // SAFETY: before Linux starts, nothing else reads the parameter.
    unsafe { amdgpu_runtime_pm = 0 };
    let mut registry = ptr::null_mut();
    // SAFETY: the caller's contract; the image is the build's, and the
    // registry's state lasts as long as what it started.
    match unsafe { stela_linux_start(&raw const stela_linux_amdgpu_image, &raw mut registry) } {
        0 => Ok(()),
        error => Err(error),
    }
}
