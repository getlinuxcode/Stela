//! The argument buffers of the suite's cases, as the Mac's runner fills
//! them without an argument encoder (`encode_arguments` in `runner.m`):
//! the Tier 2 layout, each entry's GPU address or resource ID written at
//! its offset in a shared buffer.
use super::suite::{Held, Runner};
use serde_json::Value;
use stela_driver_gpu::metal::error::{Error, Result};

/// Whether this runner fills every argument buffer of `case`.
pub fn supported(case: &Value) -> bool {
    const ARGUMENTS: [&str; 2] = ["buffer", "entries"];
    const ENTRY: [&str; 8] = [
        "format",
        "fragment",
        "function",
        "kind",
        "offset",
        "resource",
        "resource_offset",
        "vertex",
    ];
    let only = |value: &Value, keys: &[&str]| {
        value
            .as_object()
            .is_some_and(|object| object.keys().all(|key| keys.contains(&key.as_str())))
    };
    case["arguments"].as_array().into_iter().flatten().all(|a| {
        only(a, &ARGUMENTS)
            && a["entries"].as_array().into_iter().flatten().all(|e| {
                only(e, &ENTRY)
                    && matches!(e["kind"].as_str(), Some(
                            "buffer"
                                | "texture"
                                | "sampler"
                                | "icb"
                                | "compute_pipeline"
                                | "render_pipeline"
                        ))
            })
    })
}

impl Runner<'_> {
    /// Writes the case's argument buffers.
    pub(crate) fn encode_arguments(&mut self, case: &Value, held: &Held) -> Result<()> {
        for a in case["arguments"].as_array().into_iter().flatten() {
            let target = *held
                .named
                .get(a["buffer"].as_str().unwrap_or_default())
                .ok_or(Error::Invalid("an argument buffer is no buffer"))?;
            for e in a["entries"].as_array().into_iter().flatten() {
                let name = e["resource"].as_str().unwrap_or_default();
                let word = match e["kind"].as_str() {
                    Some("buffer") => {
                        let at = *held
                            .named
                            .get(name)
                            .ok_or(Error::Invalid("an argument names no buffer"))?;
                        let reference = self.device.buffer(held.buffers[at].id)?;
                        reference.address + e["resource_offset"].as_u64().unwrap_or(0)
                    }
                    Some("texture") => self.device.texture_resource_id(held.texture(name)?)?.0,
                    Some("sampler") => held.sampler(name)?.0,
                    Some("icb") => {
                        let id = held
                            .icbs
                            .iter()
                            .find(|(n, _)| n == name)
                            .map(|(_, id)| *id)
                            .ok_or(Error::Invalid("an argument names no indirect command buffer"))?;
                        self.device.indirect_command_buffer_resource_id(id)?
                    }
                    // Pipelines indirect commands set, by their resource IDs.
                    Some("compute_pipeline") => {
                        let library = case["library"].as_str().unwrap_or_default();
                        let function = e["function"].as_str().unwrap_or_default();
                        let pipeline = self.indirect_compute_pipeline(library, function)?;
                        self.device.compute_pipeline_resource_id(pipeline)?
                    }
                    Some("render_pipeline") => {
                        let pipeline = self.indirect_render_pipeline(case, e)?;
                        self.device.render_pipeline_resource_id(pipeline)?
                    }
                    _ => return Err(Error::Unsupported("an argument kind")),
                };
                let offset = e["offset"].as_u64().unwrap_or(0);
                let buffer = &held.buffers[target];
                if offset + 8 > buffer.length {
                    return Err(Error::Invalid("an argument lies past its buffer"));
                }
                // SAFETY: within the buffer's mapping, checked above.
                unsafe {
                    buffer
                        .pointer
                        .as_ptr()
                        .add(offset as usize)
                        .cast::<u64>()
                        .write_unaligned(word);
                }
            }
        }
        Ok(())
    }
}

impl Runner<'_> {
    /// A compute pipeline of `function` indirect commands can switch to.
    fn indirect_compute_pipeline(
        &mut self,
        library: &str,
        function: &str,
    ) -> Result<stela_driver_gpu::metal::device::ComputePipelineId> {
        use stela_driver_gpu::metal::device::pipelines::{
            ComputePipelineRequest, LinkedFunctions, PipelineArchives,
        };
        let library = self.library(library)?;
        let function = self.device.new_function(library, function, Some(&[]))?;
        self.device.new_compute_pipeline(&ComputePipelineRequest {
            compute_function: function,
            threadgroup_size_is_multiple_of_thread_execution_width: false,
            max_total_threads_per_threadgroup: 0,
            stage_input: None,
            linked_functions: LinkedFunctions::default(),
            support_indirect_command_buffers: true,
            archives: PipelineArchives::default(),
        })
    }

    /// The render pipeline an argument entry names: its vertex and fragment
    /// functions into one color attachment of its format.
    fn indirect_render_pipeline(
        &mut self,
        case: &Value,
        entry: &Value,
    ) -> Result<stela_driver_gpu::metal::device::RenderPipelineId> {
        use stela_driver_gpu::metal::{
            abi::PrimitiveTopologyClass,
            device::pipelines::{LinkedFunctions, PipelineArchives, RenderPipelineRequest},
            pipeline::RenderTargets,
            pixel_format::PixelFormat,
        };
        let library = self.library(case["library"].as_str().unwrap_or_default())?;
        let vertex = self.device.new_function(
            library,
            entry["vertex"].as_str().unwrap_or_default(),
            Some(&[]),
        )?;
        let fragment = self.device.new_function(
            library,
            entry["fragment"].as_str().unwrap_or_default(),
            Some(&[]),
        )?;
        let mut targets = RenderTargets::default();
        targets.color_attachments[0].pixel_format = Some(super::suite::named(
            PixelFormat::ALL,
            entry["format"].as_str().unwrap_or_default(),
        )
        .ok_or(Error::Unsupported("a pixel format this runner does not name"))?);
        self.device.new_render_pipeline(&RenderPipelineRequest {
            vertex_function: vertex,
            fragment_function: Some(fragment),
            vertex_descriptor: None,
            targets,
            input_primitive_topology: PrimitiveTopologyClass::Unspecified,
            max_vertex_amplification_count: 1,
            tessellation: Default::default(),
            vertex_linked_functions: LinkedFunctions::default(),
            fragment_linked_functions: LinkedFunctions::default(),
            support_indirect_command_buffers: true,
            archives: PipelineArchives::default(),
        })
    }
}
