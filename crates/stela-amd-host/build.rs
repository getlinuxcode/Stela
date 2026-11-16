//! With the `radv-host` feature, links libstela_radv.so from STELA_RADV_DIR
//! (by default the Meson build's target/gpu/radv-host/amd/radv_host).
use std::{env, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=STELA_RADV_DIR");
    if env::var_os("CARGO_FEATURE_RADV_HOST").is_none() {
        return;
    }
    let directory = env::var_os("STELA_RADV_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/gpu/radv-host/amd/radv_host")
        });
    let directory = directory.canonicalize().unwrap_or(directory);
    println!("cargo:rustc-link-search=native={}", directory.display());
    println!("cargo:rustc-link-lib=dylib=stela_radv");
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", directory.display());
    println!(
        "cargo:rerun-if-changed={}",
        directory.join("libstela_radv.so").display()
    );
}
