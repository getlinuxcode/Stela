//! amdgpu against Stela's Linux environment (AMD fork, J6).
//!
//! Both modes compile each of the composite's sources, as
//! `xtask/scripts/amd-kbuild.py` lists them, with the environment's flags
//! and amdgpu's own, plus what amdgpu selects that nouveau's port does not
//! and the fork's own kernel services (`src/amd/linux`).
//!
//! - The census (`amdgpu-census`) writes what stops each source and what
//!   the objects import that nothing defines to `OUT_DIR/amdgpu-census.txt`.
//! - The port (`amdgpu-port`) archives them as `stela-amdgpu`, with
//!   `stela_linux_amdgpu_image`: the environment's registrations, then the
//!   GPU buddy allocator's and amdgpu's own `module_init`. Building fails
//!   when what amdgpu reaches and nothing serves differs from [`OPEN`].
use super::native::{self, Linux};
use object::{Object, ObjectSymbol, read::archive::ArchiveFile};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Mutex,
    thread,
};
use stela_c_build::linux::Module;

const AMD: &str = "../../third_party/linux/drivers/gpu/drm/amd";
const KBUILD: &str = "build/amdgpu-kbuild.json";
const AMD_LINUX: &str = "src/amd/linux";
const LINUX: &str = "../../third_party/linux";
/// The fork's own kernel services for amdgpu (`src/amd/linux`).
const SERVICES: [&str; 4] = [
    "src/amd/linux/services",
    "src/amd/linux/pci",
    "src/amd/linux/scan",
    "src/amd/linux/trial",
];
/// What amdgpu selects that nouveau's port does not compile (Kconfig's
/// DRM_BUDDY, DRM_SUBALLOC_HELPER, DRM_CLIENT, DRM_DISPLAY_DSC_HELPER,
/// DRM_DISPLAY_HDCP_HELPER, DRM_PANEL_BACKLIGHT_QUIRKS), and amdgpu's
/// companion module amdxcp, and the library code amdgpu reaches that
/// nouveau does not, by path in Linux's tree.
const SELECTED: [&str; 15] = [
    "drivers/gpu/drm/drm_client_modeset",
    "lib/kfifo",
    "lib/bitmap-alloc",
    "lib/math/gcd",
    "mm/mempool",
    "kernel/time/timeconv",
    "kernel/locking/semaphore",
    "drivers/gpu/buddy",
    "drivers/gpu/drm/drm_buddy",
    "drivers/gpu/drm/drm_suballoc",
    "drivers/gpu/drm/drm_client",
    "drivers/gpu/drm/drm_panel_backlight_quirks",
    "drivers/gpu/drm/display/drm_dsc_helper",
    "drivers/gpu/drm/display/drm_hdcp_helper",
    "drivers/gpu/drm/amd/amdxcp/amdgpu_xcp_drv",
];
const MODULE: Module<'static> = Module {
    name: "amdgpu",
    file: "drivers/gpu/drm/amd/amdgpu/amdgpu",
};

/// What compiling the composite gave: its objects, and what stopped the
/// sources that did not compile, by object.
struct Compiled {
    #[cfg_attr(not(feature = "amdgpu-port"), allow(dead_code))]
    build: cc::Build,
    objects: Vec<PathBuf>,
    failures: BTreeMap<String, String>,
}

/// The objects to compile: the composite's, relative to amdgpu's directory,
/// then `linux:` paths in Linux's tree and `stela:` ones in this crate.
fn objects(kbuild: &Value) -> Vec<String> {
    kbuild["objects"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|o| o.as_str().map(str::to_owned))
        .chain(SELECTED.iter().map(|path| format!("linux:{path}.o")))
        .chain(SERVICES.iter().map(|path| format!("stela:{path}.o")))
        .collect()
}

fn source(object: &str) -> PathBuf {
    if let Some(path) = object.strip_prefix("linux:") {
        Path::new(LINUX).join(path)
    } else if let Some(path) = object.strip_prefix("stela:") {
        PathBuf::from(path)
    } else {
        Path::new(AMD).join("amdgpu").join(object)
    }
    .with_extension("c")
}

/// The environment's build with amdgpu's includes, defines and the fork's
/// forced header.
fn configure(linux: &Linux, kbuild: &Value, module: &Module<'_>) -> cc::Build {
    let mut includes = Vec::new();
    let mut defines = Vec::new();
    for flag in kbuild["ccflags"].as_array().into_iter().flatten() {
        let flag = flag.as_str().unwrap_or_default();
        if let Some(path) = flag.strip_prefix("-I") {
            includes.push(format!("amd/{path}"));
        } else {
            defines.push(flag.to_owned());
        }
    }
    let includes = includes.iter().map(String::as_str).collect::<Vec<_>>();
    let mut build = stela_c_build::native::configure();
    linux.environment.apply(
        &mut build,
        None,
        &native::drm_includes(&includes),
        Some(module),
    );
    stela_c_build::linux_build::upstream_warnings(&mut build);
    // The fork's answers to what Stela's configuration leaves out, before
    // the environment's headers (`src/amd/linux`).
    build
        .flag(format!("-I{AMD_LINUX}/include"))
        .flag(format!("-include{AMD_LINUX}/include/stela/amdgpu.h"));
    for flag in &defines {
        build.flag(flag);
    }
    // amdgpu prints u64 with `%Lx`, which Linux's printk (and Stela's,
    // `vsprintf.c`) reads as long long; Clang flags `L` with an integer
    // conversion on the UEFI target.
    build.flag("-Wno-format");
    build
}

/// Compile `objects` with each one's own Kbuild flags, in parallel.
fn compile(linux: &Linux, kbuild: &Value, objects: &[String], optimized: bool) -> Compiled {
    println!("cargo:rerun-if-changed={KBUILD}");
    println!("cargo:rerun-if-changed={AMD_LINUX}");
    let mut build = configure(linux, kbuild, &MODULE);
    // As Kbuild builds every profile: Linux relies on the optimizer to drop
    // the calls its configuration disables.
    if optimized {
        build.opt_level(2).cargo_metadata(false);
    }
    let compiler = build.get_compiler();
    let failures = Mutex::new(BTreeMap::new());
    let compiled = Mutex::new(Vec::new());
    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("amdgpu-objects");
    fs::create_dir_all(&out).expect("the object directory");
    let jobs = env::var("NUM_JOBS")
        .ok()
        .and_then(|j| j.parse().ok())
        .unwrap_or(8usize)
        .max(1);
    thread::scope(|scope| {
        for chunk in objects.chunks(objects.len().div_ceil(jobs)) {
            let (compiler, failures, compiled, out) = (&compiler, &failures, &compiled, &out);
            scope.spawn(move || {
                for object in chunk {
                    let mut command: Command = compiler.to_command();
                    for flag in kbuild["object_flags"][object.as_str()]
                        .as_array()
                        .into_iter()
                        .flatten()
                    {
                        command.arg(flag.as_str().unwrap_or_default());
                    }
                    let object_path = out.join(
                        object
                            .replace("linux:", "")
                            .replace("stela:", "stela_")
                            .replace("../", "")
                            .replace('/', "_"),
                    );
                    command
                        .arg("-c")
                        .arg("-ferror-limit=3")
                        .arg(source(object))
                        .arg("-o")
                        .arg(&object_path);
                    let output = command.output().expect("run the compiler");
                    if output.status.success() {
                        compiled.lock().unwrap().push(object_path);
                    } else {
                        let text = String::from_utf8_lossy(&output.stderr);
                        let first = text
                            .lines()
                            .find(|line| line.contains("error:"))
                            .unwrap_or("error without a message")
                            .to_owned();
                        failures.lock().unwrap().insert(object.clone(), first);
                    }
                }
            });
        }
    });
    let mut objects = compiled.into_inner().unwrap();
    objects.sort();
    Compiled {
        build,
        objects,
        failures: failures.into_inner().unwrap(),
    }
}

fn read_kbuild() -> Value {
    serde_json::from_str(&fs::read_to_string(KBUILD).expect("read amdgpu's Kbuild")).expect("JSON")
}

/// What stops each source, grouped by the error without its location.
fn causes(total: usize, failures: &BTreeMap<String, String>) -> String {
    let mut causes: BTreeMap<String, Vec<&String>> = BTreeMap::new();
    for (object, line) in failures {
        let cause = line
            .split("error:")
            .nth(1)
            .unwrap_or(line)
            .trim()
            .to_owned();
        causes.entry(cause).or_default().push(object);
    }
    let mut ranked: Vec<_> = causes.into_iter().collect();
    ranked.sort_by_key(|(_, objects)| core::cmp::Reverse(objects.len()));
    let mut report = format!(
        "amdgpu census: {} of {total} sources compile; {} stop on these causes\n\n",
        total - failures.len(),
        failures.len()
    );
    for (cause, objects) in &ranked {
        report += &format!("{:4}  {cause}\n      e.g. {}\n", objects.len(), objects[0]);
    }
    report
}

#[cfg(feature = "amdgpu-census")]
pub fn census(linux: &Linux) {
    let kbuild = read_kbuild();
    let objects = objects(&kbuild);
    let compiled = compile(linux, &kbuild, &objects, false);
    let mut report = causes(objects.len(), &compiled.failures);
    // What the compiled objects import that neither they nor the
    // environment's archives define: the kernel services left to supply.
    let open = open_imports(&compiled.objects, &[&linux.archive, &linux.common_archive]);
    report += &format!(
        "\n{} kernel services amdgpu imports that the environment does not define:\n",
        open.len()
    );
    for symbol in &open {
        report += &format!("  {symbol}\n");
    }
    let out = Path::new(&env::var("OUT_DIR").unwrap()).join("amdgpu-census.txt");
    fs::write(&out, report).expect("write the census");
    println!(
        "cargo:warning=amdgpu census: {} of {} sources compile, {} kernel services open; report in {}",
        objects.len() - compiled.failures.len(),
        objects.len(),
        open.len(),
        out.display()
    );
}

/// The symbols `objects` import that they and `archives` leave undefined,
/// besides the compiler's runtime and Stela's platform exports.
#[cfg(feature = "amdgpu-census")]
fn open_imports(objects: &[PathBuf], archives: &[&Path]) -> BTreeSet<String> {
    let (mut defined, mut imported) = (BTreeSet::new(), BTreeSet::new());
    for path in objects {
        let member = member(&fs::read(path).expect("read an object"));
        defined.extend(member.defined);
        imported.extend(member.imported);
    }
    for archive in archives {
        defined.extend(
            members(archive)
                .into_iter()
                .flat_map(|member| member.defined),
        );
    }
    imported
        .into_iter()
        .filter(|symbol| {
            !defined.contains(symbol)
                && !RUNTIME.contains(&symbol.as_str())
                && !symbol.starts_with(PLATFORM)
        })
        .collect()
}

#[cfg(feature = "amdgpu-port")]
mod port {
    use super::*;
    use stela_c_build::linux_capture::{self as initcalls, Captured, Projection, Scope, UnitSpec};

    /// The registrations amdgpu's image runs after the environment's, in
    /// Linux's link order: drivers/gpu's buddy allocator, then amdgpu.
    const UNITS: [UnitSpec; 2] = [
        UnitSpec {
            scope: Scope {
                slice: "gpu",
                group: "gpubuddy",
                group_ordinal: 8,
                unit_ordinal: 1,
                global_ordinal: 8,
            },
            source: "drivers/gpu/buddy",
            module: "buddy",
            file: "drivers/gpu/buddy",
        },
        UnitSpec {
            scope: Scope {
                slice: "gpu",
                group: "amdgpu",
                group_ordinal: 9,
                unit_ordinal: 1,
                global_ordinal: 9,
            },
            source: "drivers/gpu/drm/amd/amdgpu/amdgpu_drv",
            module: MODULE.name,
            file: MODULE.file,
        },
    ];

    /// The object name of a unit's source in [`objects`].
    fn object_of(spec: &UnitSpec) -> String {
        match spec.source.strip_prefix("drivers/gpu/drm/amd/amdgpu/") {
            Some(name) => format!("{name}.o"),
            None => format!("linux:{}.o", spec.source),
        }
    }

    pub fn port(linux: &Linux, mut captured: Vec<Captured>) {
        let kbuild = read_kbuild();
        let mut objects = objects(&kbuild);
        let units = UNITS.iter().map(object_of).collect::<Vec<_>>();
        objects.retain(|object| !units.contains(object));
        let compiled = compile(linux, &kbuild, &objects, true);
        assert!(
            compiled.failures.is_empty(),
            "amdgpu does not compile:\n{}",
            causes(objects.len(), &compiled.failures)
        );
        let environment_count = captured.len();
        let compiler = initcalls::compiler(&compiled.build);
        let mut expected = native::environment_units();
        for spec in UNITS {
            let placement = initcalls::Placement::new(
                &initcalls::Unit::from(spec).path(),
                &stela_c_build::linux_build::out_dir(),
                &[
                    Path::new("../.."),
                    compiler.parent().expect("compiler root"),
                ],
            )
            .expect("amdgpu source placement");
            // Each unit under its own module's name (KBUILD_MODNAME).
            let mut unit_build = configure(
                linux,
                &kbuild,
                &Module {
                    name: spec.module,
                    file: spec.file,
                },
            );
            unit_build.opt_level(2).cargo_metadata(false);
            captured.push(initcalls::compile(
                &unit_build,
                Path::new("../stela-linux/src/compat/stela/registration.h"),
                spec,
                &placement,
            ));
            expected.push(spec);
        }
        let image = initcalls::source(
            &captured,
            Projection {
                symbol: "stela_linux_amdgpu_image",
                expected: &expected,
                imported_units: environment_count,
                descriptors_only: false,
            },
        );
        let mut build = compiled.build.clone();
        build
            .objects(&compiled.objects)
            .objects(
                captured
                    .iter()
                    .skip(environment_count)
                    .map(Captured::object),
            )
            .file(image)
            .compile("stela-amdgpu");
        let archive = stela_c_build::linux_build::out_dir().join("libstela-amdgpu.a");
        check_imports(&archive, &[&linux.archive, &linux.common_archive]);
        // Hosted programs test the environment and link no driver.
        if env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "uefi") {
            println!("cargo:rustc-link-lib=static=stela-amdgpu");
        }
    }
}
#[cfg(feature = "amdgpu-port")]
pub use port::port;

/// What the compiler itself calls, which every platform's runtime supplies.
const RUNTIME: [&str; 4] = ["memcpy", "memmove", "memset", "memcmp"];
/// The prefix of what Stela's Rust side exports to the environment.
const PLATFORM: &str = "stela_linux_";
/// The prefix of the hosted test harness (`stela/test.h`).
#[cfg(feature = "amdgpu-port")]
const HARNESS: &str = "stela_test_";

/// The kernel services amdgpu reaches that Stela's Linux environment lacks,
/// or defines only as a trap, by service. Building fails when the imports
/// change, so each supplied service leaves this list with its change.
#[cfg(feature = "amdgpu-port")]
const OPEN: &[(&str, &[&str])] = &[
    // As for nouveau: TTM allocates from its own pool, and Stela never
    // swaps it out to shmem.
    (
        "shmem and page cache",
        &[
            "folio_clear_dirty_for_io",
            "mm_get_unmapped_area",
            "shmem_read_folio_gfp",
            "shmem_read_mapping_page_gfp",
            "shmem_truncate_range",
            "shmem_writeout",
        ],
    ),
    // As for nouveau: nothing tears a pseudo filesystem's superblock down.
    (
        "reopening a file without a VFS path",
        &["stela_linux_reopen_unowned_path"],
    ),
    // HDMI audio's runtime power link (an APU's USB link for amdgpu), and
    // the compute partitions amdxcp would unregister, which it never makes.
    (
        "device links and platform devices",
        &["device_link_add", "platform_device_unregister"],
    ),
    // Runtime power management, which `amdgpu::start` turns off
    // (amdgpu.runpm=0): Stela never puts the GPU to sleep. Removing the
    // device and halting it after a fatal error wait for its transactions.
    (
        "runtime power management and removal",
        &["pci_wake_from_d3", "pci_wait_for_pending_transaction"],
    ),
];

/// Compare the services the archive reaches when linked, directly or
/// through the environment's objects, and that nothing serves, with
/// [`OPEN`] (as `build/nouveau.rs` does for nouveau).
#[cfg(feature = "amdgpu-port")]
fn check_imports(port: &Path, environment: &[&Path]) {
    let port = members(port);
    let environment = environment
        .iter()
        .flat_map(|archive| members(archive))
        .collect::<Vec<_>>();
    let definers = environment
        .iter()
        .flat_map(|member| member.defined.iter().map(move |symbol| (symbol, member)))
        .collect::<BTreeMap<_, _>>();
    let defined = port
        .iter()
        .flat_map(|member| &member.defined)
        .collect::<BTreeSet<_>>();
    let traps = traps();
    let harness = env::var_os("DEP_STELA_LINUX_HOSTED_FIXTURES").is_some();
    let mut reached = port
        .iter()
        .flat_map(|member| &member.imported)
        .filter(|symbol| !defined.contains(symbol))
        .collect::<Vec<_>>();
    let mut seen = reached.iter().copied().collect::<BTreeSet<_>>();
    let mut open = BTreeSet::new();
    while let Some(symbol) = reached.pop() {
        if traps.contains(symbol) {
            open.insert(symbol.clone());
        }
        match definers.get(symbol) {
            Some(member) => reached.extend(
                member
                    .imported
                    .iter()
                    .filter(|&import| !defined.contains(import) && seen.insert(import)),
            ),
            None if RUNTIME.contains(&symbol.as_str())
                || symbol.starts_with(PLATFORM)
                || (harness && symbol.starts_with(HARNESS)) => {}
            None => {
                open.insert(symbol.clone());
            }
        }
    }
    let mut stale = Vec::new();
    for symbol in OPEN.iter().flat_map(|(_, symbols)| symbols.iter()) {
        if !open.remove(*symbol) {
            stale.push(*symbol);
        }
    }
    let report = Path::new(&env::var("OUT_DIR").unwrap()).join("amdgpu-open.txt");
    fs::write(
        &report,
        open.iter()
            .map(|symbol| format!("{symbol}\n"))
            .collect::<String>(),
    )
    .expect("write amdgpu's open services");
    assert!(
        open.is_empty() && stale.is_empty(),
        "amdgpu's imports changed: newly open {open:?}; now supplied {stale:?} (list in {})",
        report.display()
    );
}

/// What one object defines and imports.
struct Member {
    defined: BTreeSet<String>,
    imported: BTreeSet<String>,
}

fn member(data: &[u8]) -> Member {
    let object = object::File::parse(data).expect("an object file");
    let (mut defined, mut imported) = (BTreeSet::new(), BTreeSet::new());
    for symbol in object.symbols() {
        let Ok(name) = symbol.name() else {
            continue;
        };
        if symbol.is_undefined() {
            imported.insert(name.to_owned());
        } else if symbol.is_global() {
            defined.insert(name.to_owned());
        }
    }
    Member { defined, imported }
}

fn members(archive: &Path) -> Vec<Member> {
    let data = fs::read(archive).unwrap_or_else(|error| panic!("{}: {error}", archive.display()));
    let archive = ArchiveFile::parse(&*data).expect("an archive");
    archive
        .members()
        .map(|entry| {
            member(
                entry
                    .expect("an archive member")
                    .data(&*data)
                    .expect("member data"),
            )
        })
        .collect()
}

/// What the environment defines without serving: functions that trap
/// (`STELA_UNSUPPORTED`), placeholder objects (`STELA_UNSUPPORTED_OBJECT`)
/// and functions that report themselves unsupported and fail.
#[cfg(feature = "amdgpu-port")]
fn traps() -> BTreeSet<String> {
    let mut traps = BTreeSet::new();
    for source in stela_c_build::linux_build::sources("../stela-linux/src/compat/kernel")
        .into_iter()
        .chain(stela_c_build::linux_build::sources(
            "src/compat/linux/kernel",
        ))
    {
        let text = fs::read_to_string(&source).expect("read a kernel-API source");
        for (marker, field) in [
            ("STELA_UNSUPPORTED(", 1),
            ("STELA_UNSUPPORTED_OBJECT(", 1),
            ("stela_linux_unsupported(\"", 0),
        ] {
            for definition in text.split(marker).skip(1) {
                let end = definition.find([')', '"']).unwrap_or(definition.len());
                if let Some(name) = definition[..end].split(',').nth(field) {
                    traps.insert(name.trim().to_owned());
                }
            }
        }
    }
    traps
}
