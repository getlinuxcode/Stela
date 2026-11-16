//! The indirect command buffers of the suite's cases, as the Mac's runner
//! makes them (`make_icb`, `execute_icb` in `runner.m`): dispatch or draw
//! commands the CPU encodes, executed by a compute or render encoder over a
//! range or a range the GPU reads.
use super::suite::{Held, Runner, named};
use serde_json::Value;
use stela_driver_gpu::metal::{
    abi::{
        CompareFunction, CullMode, DepthClipMode, IndexType, IndirectCommandType, PrimitiveType,
        TriangleFillMode, Winding,
    },
    capabilities::Size,
    command_buffer::{
        CommandBuffer, ComputePipelineState, indirect::ExecutionRange, render::PatchSource,
    },
    device::pipelines::{ComputePipelineRequest, LinkedFunctions, PipelineArchives},
    error::{Error, Result},
    indirect::{IndirectCommandBufferDescriptor, IndirectCommandBufferId},
    pipeline::{DepthStencilDescriptor, depth_stencil},
    shader::Stage,
};

const ICB: [&str; 12] = [
    "commands",
    "count",
    "dynamic_stride",
    "inherit_buffers",
    "inherit_pipeline",
    "inherit_state",
    "max_buffers",
    "max_threadgroup_memory",
    "name",
    "reset_range",
    "type",
    "kind",
];
const COMMAND: [&str; 28] = [
    "barrier",
    "base_instance",
    "base_vertex",
    "buffers",
    "clear_barrier",
    "function",
    "imageblock",
    "index_count",
    "index_offset",
    "index_type",
    "indices",
    "instance_count",
    "mesh",
    "patches",
    "primitive",
    "reset",
    "stage_in_region",
    "stage_input",
    "state",
    "threadgroup",
    "threadgroup_memory",
    "threadgroups",
    "threads_per_grid",
    "vertex_count",
    "vertex_start",
    "kind",
    "name",
    "note",
];
const USE: [&str; 6] = [
    "attribute_stride",
    "index",
    "name",
    "offset",
    "stage",
    "kind",
];

fn only(value: &Value, keys: &[&str]) -> bool {
    value
        .as_object()
        .is_some_and(|object| object.keys().all(|key| keys.contains(&key.as_str())))
}

/// Whether this runner makes every indirect command buffer of `case`.
pub fn supported(case: &Value) -> bool {
    case["icbs"].as_array().into_iter().flatten().all(|icb| {
        only(icb, &ICB)
            && matches!(
                icb["type"].as_str(),
                Some("dispatch" | "draw" | "patches" | "mesh")
            )
            && icb["commands"]
                .as_array()
                .into_iter()
                .flatten()
                .all(|command| {
                    only(command, &COMMAND)
                        && command["buffers"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .all(|u| only(u, &USE))
                })
    })
}

/// Whether `execute` (an `execute_icb`) is one this runner encodes.
pub fn execution_supported(execute: &Value) -> bool {
    execute.is_null() || only(execute, &["count", "icb", "range_buffer", "start"])
}

fn size(value: &Value, default: u64) -> [u32; 3] {
    core::array::from_fn(|at| {
        value[at]
            .as_u64()
            .unwrap_or(if at == 0 { default } else { 1 }) as u32
    })
}

impl Runner<'_> {
    /// Makes the case's indirect command buffers into `held`, their
    /// commands encoded.
    pub(crate) fn make_icbs(&mut self, case: &Value, held: &mut Held) -> Result<()> {
        for spec in case["icbs"].as_array().into_iter().flatten() {
            let kind = spec["type"].as_str().unwrap_or("dispatch");
            let (mesh, patches) = (kind == "mesh", kind == "patches");
            let draw = mesh || patches || kind == "draw";
            let command_types = if mesh {
                IndirectCommandType::DRAW_MESH_THREADGROUPS | IndirectCommandType::DRAW_MESH_THREADS
            } else if patches {
                IndirectCommandType::DRAW_PATCHES | IndirectCommandType::DRAW_INDEXED_PATCHES
            } else if draw {
                IndirectCommandType::DRAW | IndirectCommandType::DRAW_INDEXED
            } else {
                IndirectCommandType::CONCURRENT_DISPATCH
                    | IndirectCommandType::CONCURRENT_DISPATCH_THREADS
            };
            let bindings = spec["max_buffers"].as_u64().unwrap_or(8);
            let inherit_state = spec["inherit_state"].as_bool().unwrap_or(true);
            let descriptor = IndirectCommandBufferDescriptor {
                command_types,
                inherit_pipeline_state: spec["inherit_pipeline"].as_bool().unwrap_or(draw),
                inherit_buffers: spec["inherit_buffers"] == true,
                inherit_cull_mode: inherit_state,
                inherit_depth_stencil_state: inherit_state,
                inherit_depth_bias: inherit_state,
                inherit_depth_clip_mode: inherit_state,
                inherit_front_facing_winding: inherit_state,
                inherit_triangle_fill_mode: inherit_state,
                max_vertex_buffer_bind_count: if draw { bindings } else { 0 },
                max_fragment_buffer_bind_count: if draw { bindings } else { 0 },
                max_kernel_buffer_bind_count: if draw { 0 } else { bindings },
                max_kernel_threadgroup_memory_bind_count: if draw {
                    0
                } else {
                    spec["max_threadgroup_memory"].as_u64().unwrap_or(0)
                },
                max_object_buffer_bind_count: if mesh { bindings } else { 0 },
                max_mesh_buffer_bind_count: if mesh { bindings } else { 0 },
                support_ray_tracing: false,
                support_dynamic_attribute_stride: spec["dynamic_stride"] == true,
            };
            let id = self
                .device
                .new_indirect_command_buffer(&descriptor, spec["count"].as_u64().unwrap_or(1))?;
            held.icbs
                .push((spec["name"].as_str().unwrap_or_default().to_owned(), id));
            let library = case["library"].as_str().unwrap_or_default().to_owned();
            for (index, command) in spec["commands"]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
            {
                if draw {
                    self.encode_draw(id, index, command, held)?;
                } else {
                    self.encode_dispatch(id, index, command, &library, held)?;
                }
            }
            if let Some(range) = spec["reset_range"].as_array() {
                let at = |i: usize| range.get(i).and_then(Value::as_u64).unwrap_or(0) as usize;
                self.device
                    .indirect_command_buffer_mut(id)?
                    .reset(at(0)..at(0) + at(1))?;
            }
        }
        Ok(())
    }

    fn buffer_of(
        &self,
        held: &Held,
        name: &str,
    ) -> Result<stela_driver_gpu::metal::command_buffer::BufferRef> {
        let at = *held
            .named
            .get(name)
            .ok_or(Error::Invalid("an indirect command names no buffer"))?;
        self.device.buffer(held.buffers[at].id)
    }

    fn encode_draw(
        &mut self,
        id: IndirectCommandBufferId,
        index: usize,
        command: &Value,
        held: &Held,
    ) -> Result<()> {
        let mut uses = Vec::new();
        for u in command["buffers"].as_array().into_iter().flatten() {
            let buffer = self.buffer_of(held, u["name"].as_str().unwrap_or_default())?;
            let stage = match u["stage"].as_str() {
                Some("vertex") => Stage::Vertex,
                Some("object") => Stage::Object,
                Some("mesh") => Stage::Mesh,
                _ => Stage::Fragment,
            };
            uses.push((
                stage,
                buffer,
                u["offset"].as_u64().unwrap_or(0),
                u["attribute_stride"].as_u64(),
                u["index"].as_u64().unwrap_or(0) as u32,
            ));
        }
        let number =
            |item: &Value, key: &str, default: u64| item[key].as_u64().unwrap_or(default) as u32;
        let indices = match command["indices"].as_str() {
            Some(name) => Some(self.buffer_of(held, name)?),
            None => None,
        };
        let patches = &command["patches"];
        let factors = match patches["factors"].as_str() {
            Some(name) => Some(self.buffer_of(held, name)?),
            None => None,
        };
        let point_index = match patches["control_point_index"].as_str() {
            Some(name) => Some(self.buffer_of(held, name)?),
            None => None,
        };
        let state = &command["state"];
        let depth_state = state.is_object().then(|| {
            depth_stencil(&DepthStencilDescriptor {
                depth_compare_function: named(
                    CompareFunction::ALL,
                    state["depth_compare"].as_str().unwrap_or("always"),
                )
                .unwrap_or(CompareFunction::Always),
                depth_write_enabled: state["depth_write"] == true,
                ..DepthStencilDescriptor::default()
            })
        });
        let buffer = self.device.indirect_command_buffer_mut(id)?;
        let mut r = buffer.render_command(index)?;
        for (stage, buffer, offset, stride, slot) in uses {
            match stride {
                Some(stride) => r.set_vertex_buffer_with_stride(buffer, offset, stride, slot)?,
                None => r.set_buffer(stage, buffer, offset, slot)?,
            }
        }
        if let Some(depth_state) = &depth_state {
            r.set_cull_mode(match state["cull"].as_str() {
                Some("front") => CullMode::Front,
                Some("back") => CullMode::Back,
                _ => CullMode::None,
            })?;
            r.set_front_facing_winding(if state["winding"] == "counter_clockwise" {
                Winding::CounterClockwise
            } else {
                Winding::Clockwise
            })?;
            r.set_triangle_fill_mode(if state["fill"] == "lines" {
                TriangleFillMode::Lines
            } else {
                TriangleFillMode::Fill
            })?;
            r.set_depth_clip_mode(if state["depth_clip"] == "clamp" {
                DepthClipMode::Clamp
            } else {
                DepthClipMode::Clip
            })?;
            let bias = |at: usize| state["depth_bias"][at].as_f64().unwrap_or(0.0) as f32;
            r.set_depth_bias(bias(0), bias(1), bias(2))?;
            r.set_depth_stencil_state(Some(depth_state))?;
        }
        if command["barrier"] == true {
            r.set_barrier()?;
        }
        if command["clear_barrier"] == true {
            r.clear_barrier()?;
        }
        let primitive = named(
            PrimitiveType::ALL,
            command["primitive"].as_str().unwrap_or("triangle"),
        )
        .ok_or(Error::Unsupported("a primitive type"))?;
        let mesh = &command["mesh"];
        if mesh.is_object() {
            if let Some(length) = mesh["object_threadgroup_memory"].as_u64() {
                r.set_object_threadgroup_memory_length(length, 0)?;
            }
            let (object, threads) = (
                size(&mesh["object_threads"], 1),
                size(&mesh["mesh_threads"], 1),
            );
            if mesh["threads"].is_array() {
                r.draw_mesh_threads(size(&mesh["threads"], 1), object, threads)?;
            } else {
                r.draw_mesh_threadgroups(size(&mesh["threadgroups"], 1), object, threads)?;
            }
        } else if let Some(factors) = factors {
            let source = PatchSource {
                control_points: number(patches, "control_points", 4),
                patch_indices: None,
                control_point_indices: point_index.map(|buffer| (buffer, 0)),
            };
            r.draw_patches(
                &source,
                number(patches, "patch_start", 0),
                number(patches, "patch_count", 1),
                number(patches, "instance_count", 1),
                0,
                (factors, 0, 0),
            )?;
        } else if let Some(indices) = indices {
            r.draw_indexed_primitives(
                primitive,
                number(command, "index_count", 3),
                if command["index_type"] == "uint32" {
                    IndexType::UInt32
                } else {
                    IndexType::UInt16
                },
                indices,
                command["index_offset"].as_u64().unwrap_or(0),
                number(command, "instance_count", 1),
                command["base_vertex"].as_i64().unwrap_or(0) as i32,
                number(command, "base_instance", 0),
            )?;
        } else {
            r.draw_primitives(
                primitive,
                number(command, "vertex_start", 0),
                number(command, "vertex_count", 3),
                number(command, "instance_count", 1),
                number(command, "base_instance", 0),
            )?;
        }
        // A command encoded, then reset: it does nothing.
        if command["reset"] == true {
            r.reset()?;
        }
        Ok(())
    }

    fn encode_dispatch(
        &mut self,
        id: IndirectCommandBufferId,
        index: usize,
        command: &Value,
        library: &str,
        held: &Held,
    ) -> Result<()> {
        let pipeline = match command["function"].as_str() {
            Some(function) => {
                Some(self.icb_pipeline(library, function, &command["stage_input"])?)
            }
            None => None,
        };
        let mut uses = Vec::new();
        for u in command["buffers"].as_array().into_iter().flatten() {
            uses.push((
                self.buffer_of(held, u["name"].as_str().unwrap_or_default())?,
                u["offset"].as_u64().unwrap_or(0),
                u["attribute_stride"].as_u64(),
                u["index"].as_u64().unwrap_or(0) as u32,
            ));
        }
        let buffer = self.device.indirect_command_buffer_mut(id)?;
        let mut k = buffer.compute_command(index)?;
        if let Some(pipeline) = &pipeline {
            k.set_compute_pipeline_state(pipeline)?;
        }
        for (buffer, offset, stride, slot) in uses {
            match stride {
                Some(stride) => k.set_kernel_buffer_with_stride(buffer, offset, stride, slot)?,
                None => k.set_kernel_buffer(buffer, offset, slot)?,
            }
        }
        for memory in command["threadgroup_memory"]
            .as_array()
            .into_iter()
            .flatten()
        {
            k.set_threadgroup_memory_length(
                memory["length"].as_u64().unwrap_or(16),
                memory["index"].as_u64().unwrap_or(0) as u32,
            )?;
        }
        if let Some(block) = command["imageblock"].as_array() {
            let at = |i: usize| block.get(i).and_then(Value::as_u64).unwrap_or(1);
            k.set_imageblock_size(at(0), at(1))?;
        }
        if command["stage_in_region"].is_array() {
            let region = size(&command["stage_in_region"], 1).map(u64::from);
            k.set_stage_in_region([0; 3], region)?;
        }
        if command["barrier"] == true {
            k.set_barrier()?;
        }
        if command["clear_barrier"] == true {
            k.clear_barrier()?;
        }
        let threads = size(&command["threadgroup"], 64);
        let threads = Size {
            width: u64::from(threads[0]),
            height: u64::from(threads[1]),
            depth: u64::from(threads[2]),
        };
        if command["threads_per_grid"].is_array() {
            k.concurrent_dispatch_threads(size(&command["threads_per_grid"], 1), threads)?;
        } else {
            k.concurrent_dispatch_threadgroups(size(&command["threadgroups"], 1), threads)?;
        }
        if command["reset"] == true {
            k.reset()?;
        }
        Ok(())
    }

    /// A compute pipeline indirect commands can switch to.
    fn icb_pipeline(
        &mut self,
        library: &str,
        function: &str,
        stage_input: &Value,
    ) -> Result<ComputePipelineState> {
        let library = self.library(library)?;
        let function = self.device.new_function(library, function, Some(&[]))?;
        let stage_input = if stage_input.is_object() {
            Some(super::suite::stage_input(stage_input)?)
        } else {
            None
        };
        let pipeline = self.device.new_compute_pipeline(&ComputePipelineRequest {
            compute_function: function,
            threadgroup_size_is_multiple_of_thread_execution_width: false,
            max_total_threads_per_threadgroup: 0,
            stage_input,
            linked_functions: LinkedFunctions::default(),
            support_indirect_command_buffers: true,
            archives: PipelineArchives::default(),
        })?;
        Ok(self.device.compute_pipeline_state(pipeline)?.clone())
    }

    /// Executes the indirect command buffer `execute` names in the open
    /// encoder.
    pub(crate) fn execute_icb(
        &mut self,
        execute: &Value,
        held: &Held,
        commands: &mut CommandBuffer,
    ) -> Result<()> {
        let name = execute["icb"].as_str().unwrap_or_default();
        let id = held
            .icbs
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, id)| *id)
            .ok_or(Error::Invalid(
                "an execution names no indirect command buffer",
            ))?;
        let range = match execute["range_buffer"].as_str() {
            Some(buffer) => ExecutionRange::Indirect {
                buffer: self.buffer_of(held, buffer)?,
                offset: 0,
            },
            None => {
                let start = execute["start"].as_u64().unwrap_or(0) as usize;
                ExecutionRange::Direct(
                    start..start + execute["count"].as_u64().unwrap_or(1) as usize,
                )
            }
        };
        commands.execute_commands(&mut self.device.resolver(), id, range)
    }
}
