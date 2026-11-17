//! AMD fork (J7): RADV, ACO and NIR for `stela.efi`, built as NVK is
//! (`nvk.rs`): Meson configures Mesa for the image's machine, and the
//! units RADV's library links are compiled for the image's C target
//! against `compat/libc` and archived in `target/gpu/radv`.
//!
//! What differs from NVK:
//! - no LLVM in RADV (ACO compiles every shader) and no Rust (no NAK);
//! - no libelf, so RADV's ELF path, used only with LLVM, is left out;
//! - libdrm_amdgpu, which RADV's winsys calls, compiled from
//!   `third_party/libdrm/amdgpu` as units, under `compat/mesa/drm.c`'s
//!   libdrm core, as libdrm's own library is built over libdrm;
//! - the Metal compiler's NIR options for a RADV device
//!   (`amd/radv_host/nir_options.c`, which `libstela_radv.so` also links).
use super::nvk::{self, Flavor};
use anyhow::Result;

/// The units RADV's library links, and what it needs besides.
pub(super) const RADV: Flavor = Flavor {
    name: "radv",
    options: &["-Dvulkan-drivers=amd", "-Damd-use-llvm=false"],
    drivers: &["src/amd/vulkan/libvulkan_radeon.so"],
    whole_libraries: &[
        "libvulkan_lite_runtime_a",
        "libvulkan_runtime_a",
        "libvulkan_instance_a",
        "libvulkan_radeon_so",
    ],
    skipped: &[
        "build_id",
        "u_qsort",
        "os_file_notify",
        "os_socket",
        "disk_cache",
        "mesa_cache_db",
        "u_debug_stack",
        "wsi_common_display",
        // RADV's ELF path, for LLVM's shaders: ACO's need none.
        "ac_rtld",
        "ac_rgp_elf_object_pack",
    ],
    stand_ins: &[
        "compat/mesa/build_id",
        "compat/mesa/drm",
        "compat/mesa/drm_amdgpu",
        "compat/mesa/radv_libc",
        "amd/radv_host/nir_options",
    ],
    model_unit: "radv_query.c",
    rust: false,
    libelf: false,
    libdrm_amdgpu: &[
        "amdgpu/amdgpu_bo.c",
        "amdgpu/amdgpu_cs.c",
        "amdgpu/amdgpu_device.c",
        "amdgpu/amdgpu_gpu_info.c",
        "amdgpu/amdgpu_userq.c",
        "amdgpu/amdgpu_vamgr.c",
        "amdgpu/amdgpu_vm.c",
        "amdgpu/handle_table.c",
    ],
    archives: ["libstela-radv-vulkan.a", "libstela-radv.a"],
    // Meson finds the build host's libelf when it has one.
    dropped_defines: &["USE_LIBELF"],
};

/// Build RADV's archives in `target/gpu/radv`.
pub fn build(jobs: u16) -> Result<()> {
    nvk::build_flavor(&RADV, jobs)
}
