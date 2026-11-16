//! amdgpu against Stela's Linux environment (AMD fork, J6). The census
//! (`amdgpu-census`) checks each of the composite's sources, as
//! `xtask/scripts/amd-kbuild.py` lists them, with the environment's
//! flags and amdgpu's own, and writes what stops each one to
//! `OUT_DIR/amdgpu-census.txt`: the work left before the port links.
use super::native::{self, Linux};
use serde_json::Value;
use object::{Object, ObjectSymbol, read::archive::ArchiveFile};
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
const MODULE: Module<'static> = Module {
    name: "amdgpu",
    file: "drivers/gpu/drm/amd/amdgpu/amdgpu",
};

pub fn census(linux: &Linux) {
    println!("cargo:rerun-if-changed={KBUILD}");
    let kbuild: Value =
        serde_json::from_str(&fs::read_to_string(KBUILD).expect("read amdgpu's Kbuild")).expect("JSON");
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
    linux
        .environment
        .apply(&mut build, None, &native::drm_includes(&includes), Some(&MODULE));
    stela_c_build::linux_build::upstream_warnings(&mut build);
    for flag in &defines {
        build.flag(flag);
    }
    let compiler = build.get_compiler();
    let objects: Vec<String> = kbuild["objects"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|o| o.as_str().map(str::to_owned))
        .collect();
    let failures = Mutex::new(BTreeMap::new());
    let compiled = Mutex::new(Vec::new());
    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("amdgpu-objects");
    fs::create_dir_all(&out).expect("the object directory");
    let jobs = env::var("NUM_JOBS").ok().and_then(|j| j.parse().ok()).unwrap_or(8usize).max(1);
    thread::scope(|scope| {
        for chunk in objects.chunks(objects.len().div_ceil(jobs)) {
            let (compiler, kbuild, failures, compiled, out) =
                (&compiler, &kbuild, &failures, &compiled, &out);
            scope.spawn(move || {
                for object in chunk {
                    let source = Path::new(AMD).join("amdgpu").join(object).with_extension("c");
                    let mut command: Command = compiler.to_command();
                    for flag in kbuild["object_flags"][object.as_str()].as_array().into_iter().flatten() {
                        command.arg(flag.as_str().unwrap_or_default());
                    }
                    let object_path = out.join(object.replace("../", "").replace('/', "_"));
                    command
                        .arg("-c")
                        .arg("-ferror-limit=3")
                        .arg(&source)
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
    let failures = failures.into_inner().unwrap();
    let compiled = compiled.into_inner().unwrap();
    // Group by the error, without its location.
    let mut causes: BTreeMap<String, Vec<&String>> = BTreeMap::new();
    for (object, line) in &failures {
        let cause = line.split("error:").nth(1).unwrap_or(line).trim().to_owned();
        causes.entry(cause).or_default().push(object);
    }
    let mut ranked: Vec<_> = causes.into_iter().collect();
    ranked.sort_by_key(|(_, objects)| core::cmp::Reverse(objects.len()));
    let mut report = format!(
        "amdgpu census: {} of {} sources compile; {} stop on these causes\n\n",
        objects.len() - failures.len(),
        objects.len(),
        failures.len()
    );
    for (cause, objects) in &ranked {
        report += &format!("{:4}  {cause}\n      e.g. {}\n", objects.len(), objects[0]);
    }
    // What the compiled objects import that neither they nor the
    // environment's archives define: the kernel services left to supply.
    let open = open_imports(&compiled, &[&linux.archive, &linux.common_archive]);
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
        objects.len() - failures.len(),
        objects.len(),
        open.len(),
        out.display()
    );
}

/// The symbols `objects` import that they and `archives` leave undefined,
/// besides the compiler's runtime and Stela's platform exports.
fn open_imports(objects: &[PathBuf], archives: &[&Path]) -> BTreeSet<String> {
    let (mut defined, mut imported) = (BTreeSet::new(), BTreeSet::new());
    let mut scan = |data: &[u8]| {
        let file = object::File::parse(data).expect("an object file");
        for symbol in file.symbols() {
            let Ok(name) = symbol.name() else { continue };
            if symbol.is_undefined() {
                imported.insert(name.to_owned());
            } else if symbol.is_global() {
                defined.insert(name.to_owned());
            }
        }
    };
    for path in objects {
        scan(&fs::read(path).expect("read an object"));
    }
    for archive in archives {
        let data = fs::read(archive).expect("read an archive");
        let archive = ArchiveFile::parse(&*data).expect("an archive");
        for member in archive.members() {
            let member = member.expect("an archive member");
            let file = object::File::parse(member.data(&*data).expect("member data"))
                .expect("an object file");
            for symbol in file.symbols() {
                if !symbol.is_undefined()
                    && symbol.is_global()
                    && let Ok(name) = symbol.name()
                {
                    defined.insert(name.to_owned());
                }
            }
        }
    }
    imported
        .into_iter()
        .filter(|symbol| {
            !defined.contains(symbol)
                && !["memcpy", "memmove", "memset", "memcmp"].contains(&symbol.as_str())
                && !symbol.starts_with("stela_linux_")
        })
        .collect()
}
