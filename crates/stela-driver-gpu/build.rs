#[cfg(feature = "nouveau-port")]
#[path = "build/kbuild.rs"]
mod kbuild;
#[cfg(feature = "linux-kernel")]
#[path = "build/native.rs"]
mod native;
#[cfg(feature = "nouveau-port")]
#[path = "build/nouveau.rs"]
mod nouveau;
#[path = "build/nouveau_firmware.rs"]
mod nouveau_firmware;
// AMD fork: amdgpu against the Linux environment (J6).
#[cfg(any(feature = "amdgpu-census", feature = "amdgpu-port"))]
#[path = "build/amdgpu.rs"]
mod amdgpu;

fn main() {
    nouveau_firmware::validate();
    // Active ports declare their native source inputs below. Without this
    // baseline Cargo watches every vendored file even when no C port is used.
    println!("cargo:rerun-if-changed=build.rs");
    // Clippy reads Rust and links nothing, so it needs no C objects.
    println!("cargo:rerun-if-env-changed=CLIPPY_ARGS");
    #[cfg(feature = "linux-kernel")]
    if std::env::var_os("CLIPPY_ARGS").is_some() {
        let tests = stela_c_build::linux_build::Tests::default();
        tests.write();
        return;
    }
    #[cfg(feature = "linux-kernel")]
    let (linux, mut tests) = (
        native::Linux::new(),
        stela_c_build::linux_build::Tests::default(),
    );
    #[cfg(feature = "linux-kernel")]
    linux.tests(&mut tests);
    #[cfg(feature = "nouveau-port")]
    let mut linux = linux;
    // AMD fork: amdgpu's image starts from the same environment registrations.
    #[cfg(feature = "amdgpu-port")]
    let environment = linux.captured.clone();
    #[cfg(feature = "nouveau-port")]
    nouveau::port(&mut linux);
    #[cfg(feature = "amdgpu-port")]
    amdgpu::port(&linux, environment);
    #[cfg(feature = "amdgpu-census")]
    amdgpu::census(&linux);
    #[cfg(feature = "linux-kernel")]
    tests.write();
}
