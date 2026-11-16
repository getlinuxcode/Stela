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
    abi::{ArgumentBuffersTier, DispatchType, GpuFamily, IndexType, ReadWriteTextureTier},
    capabilities::{Capabilities, Size},
    command_buffer::{CommandBuffer, ComputePipelineState},
    device::{
        BufferId, LibraryId, SamplerId,
        pipelines::{ComputePipelineRequest, LinkedFunctions, PipelineArchives},
    },
    error::{Error, Result},
    vertex::{Attribute, StageInLayout, StageInStep, StageInputDescriptor, VertexFormat},
};

const BASIC_CASE: [&str; 17] = [
    "buffers",
    "constants",
    "stage_input",
    "dispatch",
    "function",
    "grid",
    "informational",
    "kind",
    "library",
    "name",
    "note",
    "outputs",
    "requires",
    "samplers",
    "textures",
    "threadgroup",
    "timeout",
];
/// `storage` and `write_combined` change where the CPU reaches a buffer,
/// not what it holds.
const BASIC_BUFFER: [&str; 6] = [
    "file",
    "index",
    "length",
    "name",
    "storage",
    "write_combined",
];
/// Besides the bytes, what the check alone reads.
pub(crate) const BASIC_OUTPUT: [&str; 11] = [
    "dtype",
    "expect",
    "expect_alternatives",
    "expect_mask",
    "expect_tolerance",
    "length",
    "mask",
    "name",
    "stride",
    "tolerance",
    "sorted_compare",
];

/// Whether the runner reproduces `case` exactly.
fn basic(case: &Value) -> bool {
    let only = |value: &Value, keys: &[&str]| {
        value
            .as_object()
            .is_some_and(|object| object.keys().all(|key| keys.contains(&key.as_str())))
    };
    let all = |field: &str, keys: &[&str]| {
        case[field].is_null()
            || case[field]
                .as_array()
                .is_some_and(|items| items.iter().all(|item| only(item, keys)))
    };
    case["kind"] == "compute"
        && only(case, &BASIC_CASE)
        && all("buffers", &BASIC_BUFFER)
        && all("outputs", &BASIC_OUTPUT)
        && all("textures", &super::suite_resources::TEXTURE)
        && case["textures"].as_array().into_iter().flatten().all(|t| {
            super::suite_resources::texture_type(t["type"].as_str().unwrap_or("2d")).is_some()
        })
        && all("samplers", &super::suite_resources::SAMPLER)
        && (case["dispatch"].is_null() || case["dispatch"] == "threads")
        && (case["stage_input"].is_null()
            || only(
                &case["stage_input"],
                &["attributes", "index_buffer", "index_type", "layouts"],
            ))
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
        let render = super::suite_render::basic(case);
        if !(basic(case) || render)
            || !(only.is_empty() || only.iter().any(|text| name.contains(text)))
        {
            continue;
        }
        let out = results.join(name);
        if out.join("status.json").exists() {
            skipped += 1;
            continue;
        }
        fs::create_dir_all(&out).map_err(|error| error.to_string())?;
        // A crash (a driver abort) leaves this status, and the next run goes on.
        let crashed = json!({"status": "failed", "error": "the runner crashed"});
        fs::write(out.join("status.json"), crashed.to_string()).map_err(|e| e.to_string())?;
        let begun = Instant::now();
        let ran = if render {
            runner.render_case(case, &out)
        } else {
            runner.case(case, &out)
        };
        let status = match ran {
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

pub(crate) struct Runner<'a> {
    pub(crate) device: &'a mut MetalDevice,
    pub(crate) exporter: &'a Exporter,
    pub(crate) plan_dir: PathBuf,
    libraries: BTreeMap<String, LibraryId>,
    /// By library, function, threadgroup size, and the constants and stage
    /// input the case gives, as JSON.
    pipelines: BTreeMap<(String, String, [u64; 3], String), ComputePipelineState>,
    /// The next fake guest range index each import takes.
    pub(crate) next_import: u64,
}

/// The value of `all` the runner names `name`: its Rust name, compared
/// without case or underscores (`bgr10_xr` is `Bgr10Xr`, `source_alpha`
/// `SourceAlpha`).
pub(crate) fn named<T: Copy + core::fmt::Debug>(all: &[T], name: &str) -> Option<T> {
    let plain = |text: &str| -> String {
        text.chars()
            .filter(|c| *c != '_')
            .flat_map(char::to_lowercase)
            .collect()
    };
    let name = plain(name);
    all.iter()
        .copied()
        .find(|value| plain(&format!("{value:?}")) == name)
}

/// A shared buffer a case uses, and its mapping.
pub(crate) struct Shared {
    pub(crate) id: BufferId,
    pub(crate) pointer: core::ptr::NonNull<u8>,
    pub(crate) length: u64,
}

impl Shared {
    /// Its first `length` bytes, which the GPU finished writing.
    pub(crate) fn bytes(&self, length: u64) -> &[u8] {
        // SAFETY: the export maps `self.length` bytes, which outlive self.
        unsafe {
            core::slice::from_raw_parts(self.pointer.as_ptr(), length.min(self.length) as usize)
        }
    }
}

/// What a case made, which [`Runner::release`] frees.
#[derive(Default)]
pub(crate) struct Held {
    pub(crate) buffers: Vec<Shared>,
    /// The case's buffers by name: an index into `buffers`.
    pub(crate) named: BTreeMap<String, usize>,
    pub(crate) textures: Vec<(String, super::suite_resources::Texture)>,
    pub(crate) samplers: Vec<SamplerId>,
}

/// A kernel's stage input, as the runner's `stage_input_descriptor`.
fn stage_input(spec: &Value) -> Result<StageInputDescriptor> {
    let number = |item: &Value, key: &str, default: u64| item[key].as_u64().unwrap_or(default);
    let mut attributes = Vec::new();
    for item in spec["attributes"].as_array().into_iter().flatten() {
        attributes.push(Attribute {
            index: number(item, "index", 0) as u32,
            format: named(
                VertexFormat::ALL,
                item["format"].as_str().unwrap_or_default(),
            )
            .ok_or(Error::Unsupported("an attribute format"))?,
            offset: number(item, "offset", 0),
            buffer_index: number(item, "buffer", 0) as u32,
        });
    }
    let mut layouts = Vec::new();
    for item in spec["layouts"].as_array().into_iter().flatten() {
        let step = item["step"].as_str().unwrap_or("grid_x");
        let step_function = match step {
            "grid_x" => StageInStep::ThreadPositionInGridX,
            "grid_y" => StageInStep::ThreadPositionInGridY,
            "grid_x_indexed" => StageInStep::ThreadPositionInGridXIndexed,
            "grid_y_indexed" => StageInStep::ThreadPositionInGridYIndexed,
            "constant" => StageInStep::Constant,
            _ => return Err(Error::Unsupported("a stage-in step function")),
        };
        if item["stride"].is_string() {
            return Err(Error::Unsupported("a dynamic stage-in stride"));
        }
        layouts.push(StageInLayout {
            buffer_index: number(item, "buffer", 0) as u32,
            stride: number(item, "stride", 16),
            step_function,
            step_rate: if step == "constant" {
                0
            } else {
                number(item, "rate", 1)
            },
        });
    }
    Ok(StageInputDescriptor {
        attributes,
        layouts,
        index_type: if spec["index_type"] == "uint16" {
            IndexType::UInt16
        } else {
            IndexType::UInt32
        },
        index_buffer_index: number(spec, "index_buffer", 0) as u32,
    })
}

/// Whether the device has what `requires` names, as the runner's `unmet`:
/// the missing one, if any.
pub(crate) fn unmet(capabilities: &Capabilities, requires: &Value) -> Option<String> {
    for family in requires["families"].as_array().into_iter().flatten() {
        let name = family.as_str().unwrap_or_default();
        if !named(GpuFamily::ALL, name).is_some_and(|family| capabilities.supports_family(family)) {
            return Some(format!("GPU family {name}"));
        }
    }
    for feature in requires["features"].as_array().into_iter().flatten() {
        let name = feature.as_str().unwrap_or_default();
        let c = capabilities;
        let has = match name {
            "function_pointers" => c.supports_function_pointers,
            "function_pointers_render" => c.supports_function_pointers_from_render,
            "raytracing" => c.supports_raytracing,
            "dynamic_libraries" => c.supports_dynamic_libraries,
            "render_dynamic_libraries" => c.supports_render_dynamic_libraries,
            "float32_filtering" => c.supports_32bit_float_filtering,
            "float32_msaa" => c.supports_32bit_msaa,
            "bc_compression" => c.supports_bc_texture_compression,
            "query_texture_lod" => c.supports_query_texture_lod,
            "barycentrics" => c.supports_shader_barycentric_coordinates,
            "pull_model_interpolation" => c.supports_pull_model_interpolation,
            "raster_order_groups" => c.raster_order_groups,
            "programmable_sample_positions" => c.programmable_sample_positions,
            "depth24_stencil8" => c.depth24_stencil8_supported,
            "argument_buffers_tier2" => c.argument_buffers_tier == ArgumentBuffersTier::Tier2,
            "read_write_textures_tier2" => c.read_write_texture_tier == ReadWriteTextureTier::Tier2,
            "primitive_motion_blur" => c.supports_primitive_motion_blur,
            "memoryless_render_targets" => c.memoryless_render_targets,
            "framebuffer_reads" => c.framebuffer_reads,
            "simd_reduction" => {
                c.supports_family(GpuFamily::Apple7) || c.supports_family(GpuFamily::Mac2)
            }
            // Counters, logging, amplification: this runner does not check them.
            _ => false,
        };
        if !has {
            return Some(format!("feature {name}"));
        }
    }
    None
}

impl Runner<'_> {
    /// A shared buffer of `length` bytes (at least 16) holding `data`.
    pub(crate) fn shared(&mut self, data: &[u8], length: u64) -> Result<Shared> {
        let length = length.max(data.len() as u64).max(16);
        let index = self.next_import;
        self.next_import += 1;
        let (id, pointer) = shared_buffer(self.device, self.exporter, index, length)?;
        // SAFETY: the export maps at least `length` bytes, zeroed.
        unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), pointer.as_ptr(), data.len()) };
        Ok(Shared {
            id,
            pointer,
            length,
        })
    }

    /// The bytes of a buffer's or texture's `file`, none without one.
    pub(crate) fn input(&self, spec: &Value) -> Result<Vec<u8>> {
        match spec["file"].as_str() {
            Some(file) => fs::read(self.plan_dir.join(file))
                .map_err(|_| Error::Invalid("an input file is missing")),
            None => Ok(Vec::new()),
        }
    }

    /// Makes the case's buffers, by name.
    pub(crate) fn make_buffers(&mut self, specs: &Value, held: &mut Held) -> Result<()> {
        for spec in specs.as_array().into_iter().flatten() {
            let data = self.input(spec)?;
            let shared = self.shared(&data, spec["length"].as_u64().unwrap_or(16))?;
            let name = spec["name"].as_str().unwrap_or_default().to_owned();
            held.named.insert(name, held.buffers.len());
            held.buffers.push(shared);
        }
        Ok(())
    }

    /// Writes each output: a buffer's declared length, a texture's every
    /// level of every slice with tight rows, as the runner's `store_outputs`.
    pub(crate) fn store_outputs(
        &self,
        case: &Value,
        held: &Held,
        readbacks: &[(String, usize)],
        out: &Path,
    ) -> Result<()> {
        for output in case["outputs"].as_array().into_iter().flatten() {
            let name = output["name"].as_str().unwrap_or_default();
            let bytes = if let Some((_, at)) = readbacks.iter().find(|r| r.0 == name) {
                let buffer = &held.buffers[*at];
                buffer.bytes(buffer.length)
            } else if let Some(&at) = held.named.get(name) {
                let buffer = &held.buffers[at];
                buffer.bytes(output["length"].as_u64().unwrap_or(buffer.length))
            } else {
                return Err(Error::Invalid(
                    "an output is neither a buffer nor a texture",
                ));
            };
            fs::write(out.join(format!("{name}.bin")), bytes)
                .map_err(|_| Error::Invalid("cannot write an output"))?;
        }
        Ok(())
    }

    /// Records the readback of each texture output; their buffers' indices.
    pub(crate) fn read_back_outputs(
        &mut self,
        case: &Value,
        held: &mut Held,
        commands: &mut CommandBuffer,
    ) -> Result<Vec<(String, usize)>> {
        let mut readbacks = Vec::new();
        let names: Vec<String> = case["outputs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|o| o["name"].as_str().map(str::to_owned))
            .filter(|name| held.textures.iter().any(|(t, _)| t == name))
            .collect();
        if names.is_empty() {
            return Ok(readbacks);
        }
        commands.begin_blit()?;
        for name in names {
            let at = held
                .textures
                .iter()
                .position(|(t, _)| *t == name)
                .expect("named above");
            let readback = self.read_back(&held.textures[at].1, commands)?;
            readbacks.push((name, held.buffers.len()));
            held.buffers.push(readback);
        }
        commands.end_encoding(&mut self.device.resolver())?;
        Ok(readbacks)
    }

    /// Frees what a case made, and retires the imports so their pages unmap.
    pub(crate) fn release(&mut self, held: Held) {
        for buffer in held.buffers {
            let _ = self.device.destroy_buffer(buffer.id);
        }
        for (_, texture) in held.textures {
            let _ = self.device.destroy_texture(texture.id);
        }
        for sampler in held.samplers {
            let _ = self.device.destroy_sampler(sampler);
        }
        let _ = self.device.poll();
        while self.device.retired().is_some() {}
    }

    /// Runs one compute case; the GPU's milliseconds.
    fn case(&mut self, case: &Value, out: &Path) -> Result<f64> {
        if let Some(missing) = unmet(self.device.capabilities(), &case["requires"]) {
            let _ = missing;
            return Err(Error::Unsupported("a family or feature the device lacks"));
        }
        let mut held = Held::default();
        let result = self.compute_inner(case, out, &mut held);
        self.release(held);
        result
    }

    fn compute_inner(&mut self, case: &Value, out: &Path, held: &mut Held) -> Result<f64> {
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
        let state = self.pipeline(library, function, group, case)?;

        let mut commands = self.device.new_command_buffer();
        self.make_buffers(&case["buffers"], held)?;
        held.textures = self.make_textures(&case["textures"], &mut commands, &mut held.buffers)?;
        let mut samplers = Vec::new();
        for spec in case["samplers"].as_array().into_iter().flatten() {
            let (id, resource) = self.make_sampler(spec)?;
            held.samplers.push(id);
            samplers.push((resource, spec["index"].as_u64().unwrap_or(0) as u32));
        }
        commands.begin_compute(DispatchType::Serial)?;
        commands.set_compute_pipeline_state(&state)?;
        for spec in case["buffers"].as_array().into_iter().flatten() {
            let name = spec["name"].as_str().unwrap_or_default();
            let buffer = &held.buffers[held.named[name]];
            let Some(slot) = spec["index"].as_u64() else {
                continue;
            };
            commands.set_compute_buffer(Some(self.device.buffer(buffer.id)?), 0, slot as u32)?;
        }
        for (_, texture) in &held.textures {
            if let Some(index) = texture.index {
                self.device.note_texture_binding(&mut commands, texture.id);
                let resource = self.device.texture_resource_id(texture.id)?;
                commands.set_compute_texture(resource, index)?;
            }
        }
        for (resource, index) in samplers {
            commands.set_compute_sampler(&mut self.device.resolver(), resource, None, index)?;
        }
        let threads = Size {
            width: group[0],
            height: group[1],
            depth: group[2],
        };
        if case["dispatch"] == "threads" {
            if case["stage_input"].is_object() {
                commands.set_stage_in_region([0; 3], grid)?;
            }
            let grid = grid.map(|size| size as u32);
            commands.dispatch_threads(&mut self.device.resolver(), grid, threads)?;
        } else {
            let groups = [0, 1, 2].map(|axis| grid[axis].div_ceil(group[axis]) as u32);
            commands.dispatch_threadgroups(&mut self.device.resolver(), groups, threads)?;
        }
        commands.end_encoding(&mut self.device.resolver())?;
        let readbacks = self.read_back_outputs(case, held, &mut commands)?;
        let started = Instant::now();
        let serial = self.device.commit(commands)?;
        wait(self.device, serial)?;
        let gpu_ms = started.elapsed().as_secs_f64() * 1e3;
        self.store_outputs(case, held, &readbacks, out)?;
        Ok(gpu_ms)
    }

    /// The Metal library the Mac compiled as `name`, loaded once.
    pub(crate) fn library(&mut self, name: &str) -> Result<LibraryId> {
        if let Some(&id) = self.libraries.get(name) {
            return Ok(id);
        }
        let path = self.plan_dir.join("lib").join(format!("{name}.metallib"));
        // The suite compiles its *_runtime_* libraries from MSL source as the
        // case runs, which only a Mac's compiler does.
        let bytes = fs::read(&path)
            .map_err(|_| Error::Unsupported("a library compiled from source at run time"))?;
        let id = self.device.new_library(bytes)?;
        self.libraries.insert(name.to_owned(), id);
        Ok(id)
    }

    fn pipeline(
        &mut self,
        library: &str,
        function: &str,
        group: [u64; 3],
        case: &Value,
    ) -> Result<ComputePipelineState> {
        let shape = format!("{} {}", case["constants"], case["stage_input"]);
        let key = (library.to_owned(), function.to_owned(), group, shape);
        if let Some(state) = self.pipelines.get(&key) {
            return Ok(state.clone());
        }
        let library = self.library(library)?;
        let constants = super::suite_constants::values(&case["constants"])?;
        let function = self
            .device
            .new_function(library, function, Some(&constants))?;
        let stage_input = if case["stage_input"].is_object() {
            Some(stage_input(&case["stage_input"])?)
        } else {
            None
        };
        let pipeline = self.device.new_compute_pipeline(&ComputePipelineRequest {
            compute_function: function,
            threadgroup_size_is_multiple_of_thread_execution_width: false,
            max_total_threads_per_threadgroup: 0,
            stage_input,
            linked_functions: LinkedFunctions::default(),
            support_indirect_command_buffers: false,
            archives: PipelineArchives::default(),
        })?;
        let state = self.device.compute_pipeline_state(pipeline)?.clone();
        self.pipelines.insert(key, state.clone());
        Ok(state)
    }
}
