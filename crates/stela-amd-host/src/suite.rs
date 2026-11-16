//! `--suite DIR`: runs the Metal suite's compute cases on the Metal device,
//! as the Mac's runner does (`crates/stela-driver-gpu/tests/metal_suite/
//! runner.m`), and writes their results where `cargo xtask gpu metal-suite
//! check` reads them: `DIR/results/linux-gfx1201/CASE/{status.json,
//! OUTPUT.bin}`.
//!
//! It runs the cases that name only a library, a function, buffers (from a
//! file, or zeroed), a grid and a threadgroup: `dispatchThreadgroups` over
//! the grid rounded up to whole threadgroups, then each output buffer back.
//! Other cases are left unrun. A case that already has a status is skipped,
//! so a run a crash stopped resumes.
use super::device::{Exporter, MetalDevice, shared_buffer, wait};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::Instant,
};
use stela_driver_gpu::metal::{
    abi::DispatchType,
    capabilities::Size,
    command_buffer::ComputePipelineState,
    device::{
        BufferId, LibraryId,
        pipelines::{ComputePipelineRequest, LinkedFunctions, PipelineArchives},
    },
    error::{Error, Result},
};

const BASIC_CASE: [&str; 8] = [
    "buffers",
    "function",
    "grid",
    "kind",
    "library",
    "name",
    "outputs",
    "threadgroup",
];
const BASIC_BUFFER: [&str; 4] = ["file", "index", "length", "name"];
const BASIC_OUTPUT: [&str; 6] = [
    "dtype",
    "expect",
    "expect_tolerance",
    "length",
    "name",
    "tolerance",
];

/// Whether the runner reproduces `case` exactly.
fn basic(case: &Value) -> bool {
    let only = |value: &Value, keys: &[&str]| {
        value
            .as_object()
            .is_some_and(|object| object.keys().all(|key| keys.contains(&key.as_str())))
    };
    let all = |field: &str, keys: &[&str]| {
        case[field]
            .as_array()
            .is_some_and(|items| items.iter().all(|item| only(item, keys)))
    };
    case["kind"] == "compute"
        && only(case, &BASIC_CASE)
        && all("buffers", &BASIC_BUFFER)
        && all("outputs", &BASIC_OUTPUT)
}

/// Runs the selected cases of the suite in `dir`; `only` keeps the cases
/// whose names contain one of its texts.
pub fn run(
    device: &mut MetalDevice,
    exporter: &Exporter,
    dir: &Path,
    only: &[String],
) -> core::result::Result<(), String> {
    let plan_dir = dir.join("plan");
    let text = fs::read_to_string(plan_dir.join("plan.json"))
        .map_err(|error| format!("{}: {error}", plan_dir.join("plan.json").display()))?;
    let plan: Value = serde_json::from_str(&text).map_err(|error| error.to_string())?;
    let results = dir.join("results/linux-gfx1201");
    let mut runner = Runner {
        device,
        exporter,
        plan_dir,
        libraries: BTreeMap::new(),
        pipelines: BTreeMap::new(),
        next_import: 1 << 8,
    };
    let (mut ok, mut failed, mut skipped) = (0, 0, 0);
    let started = Instant::now();
    let cases = plan["cases"].as_array().map_or(&[][..], Vec::as_slice);
    for case in cases {
        let name = case["name"].as_str().unwrap_or_default();
        if !basic(case) || !(only.is_empty() || only.iter().any(|text| name.contains(text))) {
            continue;
        }
        let out = results.join(name);
        if out.join("status.json").exists() {
            skipped += 1;
            continue;
        }
        fs::create_dir_all(&out).map_err(|error| error.to_string())?;
        let begun = Instant::now();
        let status = match runner.case(case, &out) {
            Ok(gpu_ms) => {
                ok += 1;
                json!({"status": "ok", "gpu_ms": gpu_ms, "seconds": begun.elapsed().as_secs_f64()})
            }
            Err(Error::DeviceLost) => {
                return Err(format!("{name}: the device was lost"));
            }
            Err(error) => {
                failed += 1;
                let kind = if matches!(error, Error::Unsupported(_)) {
                    "unsupported"
                } else {
                    "failed"
                };
                json!({"status": kind, "error": format!("{error:?}")})
            }
        };
        fs::write(out.join("status.json"), status.to_string()).map_err(|e| e.to_string())?;
        if (ok + failed) % 500 == 0 {
            println!("  {} cases run ({:.0?})", ok + failed, started.elapsed());
        }
    }
    println!(
        "  suite: {ok} cases ran, {failed} failed or unsupported, {skipped} already done, in \
         {:.1?}; results in {}",
        started.elapsed(),
        results.display()
    );
    Ok(())
}

struct Runner<'a> {
    device: &'a mut MetalDevice,
    exporter: &'a Exporter,
    plan_dir: PathBuf,
    libraries: BTreeMap<String, LibraryId>,
    /// By library, function and threadgroup width, height and depth.
    pipelines: BTreeMap<(String, String, [u64; 3]), ComputePipelineState>,
    /// The next fake guest range index each import takes.
    next_import: u64,
}

impl Runner<'_> {
    /// Runs one case; the GPU's milliseconds.
    fn case(&mut self, case: &Value, out: &Path) -> Result<f64> {
        let library = case["library"].as_str().unwrap_or_default();
        let function = case["function"].as_str().unwrap_or_default();
        let triple = |field: &str, default: u64| -> [u64; 3] {
            let item = |at: usize| {
                case[field][at]
                    .as_u64()
                    .unwrap_or(if at == 0 { default } else { 1 })
            };
            [item(0), item(1), item(2)]
        };
        let grid = triple("grid", 1);
        let group = triple("threadgroup", 64);
        let state = self.pipeline(library, function, group)?;

        let mut buffers: Vec<(String, u32, BufferId, core::ptr::NonNull<u8>, u64)> = Vec::new();
        let result = (|| {
            for spec in case["buffers"].as_array().into_iter().flatten() {
                let data = match spec["file"].as_str() {
                    Some(file) => fs::read(self.plan_dir.join(file))
                        .map_err(|_| Error::Invalid("an input file is missing"))?,
                    None => Vec::new(),
                };
                let length = spec["length"]
                    .as_u64()
                    .unwrap_or(16)
                    .max(data.len() as u64)
                    .max(16);
                let index = self.next_import;
                self.next_import += 1;
                let (buffer, pointer) = shared_buffer(self.device, self.exporter, index, length)?;
                // SAFETY: the export maps at least `length` bytes, zeroed.
                unsafe {
                    core::ptr::copy_nonoverlapping(data.as_ptr(), pointer.as_ptr(), data.len());
                }
                let slot = spec["index"].as_u64().unwrap_or(0) as u32;
                let name = spec["name"].as_str().unwrap_or_default().to_owned();
                buffers.push((name, slot, buffer, pointer, length));
            }
            let mut commands = self.device.new_command_buffer();
            commands.begin_compute(DispatchType::Serial)?;
            commands.set_compute_pipeline_state(&state)?;
            for &(_, slot, buffer, ..) in &buffers {
                commands.set_compute_buffer(Some(self.device.buffer(buffer)?), 0, slot)?;
            }
            let groups = [0, 1, 2].map(|axis| grid[axis].div_ceil(group[axis]) as u32);
            let threads = Size {
                width: group[0],
                height: group[1],
                depth: group[2],
            };
            commands.dispatch_threadgroups(&mut self.device.resolver(), groups, threads)?;
            commands.end_encoding(&mut self.device.resolver())?;
            let started = Instant::now();
            let serial = self.device.commit(commands)?;
            wait(self.device, serial)?;
            let gpu_ms = started.elapsed().as_secs_f64() * 1e3;
            for output in case["outputs"].as_array().into_iter().flatten() {
                let name = output["name"].as_str().unwrap_or_default();
                let Some((_, _, _, pointer, length)) = buffers.iter().find(|b| b.0 == name) else {
                    continue;
                };
                // As the Mac's runner, the output's declared length when it
                // has one: buffers take at least 16 bytes.
                let length = output["length"]
                    .as_u64()
                    .map_or(*length, |declared| declared.min(*length));
                // SAFETY: the GPU finished writing the mapping.
                let bytes =
                    unsafe { core::slice::from_raw_parts(pointer.as_ptr(), length as usize) };
                fs::write(out.join(format!("{name}.bin")), bytes)
                    .map_err(|_| Error::Invalid("cannot write an output"))?;
            }
            Ok(gpu_ms)
        })();
        for (_, _, buffer, ..) in buffers {
            let _ = self.device.destroy_buffer(buffer);
        }
        // Retire the freed imports so their pages unmap.
        let _ = self.device.poll();
        while self.device.retired().is_some() {}
        result
    }

    fn pipeline(
        &mut self,
        library: &str,
        function: &str,
        group: [u64; 3],
    ) -> Result<ComputePipelineState> {
        let key = (library.to_owned(), function.to_owned(), group);
        if let Some(state) = self.pipelines.get(&key) {
            return Ok(state.clone());
        }
        let library = match self.libraries.get(library) {
            Some(&id) => id,
            None => {
                let path = self
                    .plan_dir
                    .join("lib")
                    .join(format!("{library}.metallib"));
                // The suite compiles its *_runtime_* libraries from MSL source
                // as the case runs, which only a Mac's compiler does.
                let bytes = fs::read(&path).map_err(|_| {
                    Error::Unsupported("a library compiled from source at run time")
                })?;
                let id = self.device.new_library(bytes)?;
                self.libraries.insert(library.to_owned(), id);
                id
            }
        };
        let function = self.device.new_function(library, function, Some(&[]))?;
        let pipeline = self.device.new_compute_pipeline(&ComputePipelineRequest {
            compute_function: function,
            threadgroup_size_is_multiple_of_thread_execution_width: false,
            max_total_threads_per_threadgroup: 0,
            stage_input: None,
            linked_functions: LinkedFunctions::default(),
            support_indirect_command_buffers: false,
            archives: PipelineArchives::default(),
        })?;
        let state = self.device.compute_pipeline_state(pipeline)?.clone();
        self.pipelines.insert(key, state.clone());
        Ok(state)
    }
}
