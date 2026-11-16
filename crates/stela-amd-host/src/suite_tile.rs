//! Tile shading in the suite's render cases, as the Mac's runner encodes
//! it: tile pipelines over the pass's color attachments, dispatched per
//! tile before the draws or after them, in order, with their tile-stage
//! buffers, textures and threadgroup memory.
use super::suite::{Held, Runner};
use serde_json::Value;
use stela_driver_gpu::metal::{
    capabilities::Size,
    command_buffer::{CommandBuffer, ComputePipelineState},
    device::pipelines::{ComputePipelineRequest, LinkedFunctions, PipelineArchives},
    error::Result,
    pass::TileMemory,
    pipeline::{MAX_COLOR_ATTACHMENTS, TileTarget, tile_target},
    pixel_format::PixelFormat,
};

/// What a tile dispatch may say.
pub const TILE: [&str; 7] = [
    "before",
    "function",
    "immutable_buffers",
    "matches_tile_size",
    "max_total_threads",
    "threadgroup_memory",
    "threads",
];

/// The pass's tile size and memory.
pub fn tile_memory(render: &Value) -> TileMemory {
    TileMemory {
        width: render["tile_size"][0].as_u64().unwrap_or(0),
        height: render["tile_size"][1].as_u64().unwrap_or(0),
        threadgroup_memory_length: render["tile_memory"].as_u64().unwrap_or(0),
        imageblock_sample_length: render["imageblock_sample_length"].as_u64().unwrap_or(0),
    }
}

/// A tile pipeline the pass dispatches.
pub struct TileDispatch {
    state: ComputePipelineState,
    target: TileTarget,
    spec: Value,
}

impl Runner<'_> {
    /// The tile pipelines of `render`'s tiles over attachments of `formats`.
    pub(crate) fn tile_pipelines(
        &mut self,
        render: &Value,
        library: stela_driver_gpu::metal::device::LibraryId,
        constants: &[stela_driver_gpu::metal::device::pipelines::ConstantValue],
        formats: &[Option<PixelFormat>; MAX_COLOR_ATTACHMENTS],
    ) -> Result<Vec<TileDispatch>> {
        let mut out = Vec::new();
        for spec in render["tiles"].as_array().into_iter().flatten() {
            let function = self.device.new_function(
                library,
                spec["function"].as_str().unwrap_or_default(),
                Some(constants),
            )?;
            let target = tile_target(
                self.device.capabilities(),
                formats,
                render["samples"].as_u64().unwrap_or(1),
                spec["matches_tile_size"] == true,
            )?;
            let pipeline = self.device.new_tile_pipeline(
                &ComputePipelineRequest {
                    compute_function: function,
                    threadgroup_size_is_multiple_of_thread_execution_width: false,
                    max_total_threads_per_threadgroup: spec["max_total_threads"].as_u64().unwrap_or(0),
                    stage_input: None,
                    linked_functions: LinkedFunctions::default(),
                    support_indirect_command_buffers: false,
                    archives: PipelineArchives::default(),
                },
                target,
            )?;
            let (state, target) = self.device.tile_pipeline_state(pipeline)?;
            out.push(TileDispatch {
                state: state.clone(),
                target,
                spec: spec.clone(),
            });
        }
        Ok(out)
    }

    /// Binds the case's tile-stage resources.
    pub(crate) fn bind_tile_resources(
        &mut self,
        case: &Value,
        held: &Held,
        commands: &mut CommandBuffer,
    ) -> Result<()> {
        let tile = |item: &&Value| item["stage"] == "tile";
        for item in case["buffers"].as_array().into_iter().flatten().filter(tile) {
            let Some(index) = item["index"].as_u64() else { continue };
            let at = held.named[item["name"].as_str().unwrap_or_default()];
            let reference = self.device.buffer(held.buffers[at].id)?;
            commands.set_tile_buffer(
                Some(reference),
                item["offset"].as_u64().unwrap_or(0),
                index as u32,
            )?;
        }
        for item in case["textures"].as_array().into_iter().flatten().filter(tile) {
            let Some(index) = item["index"].as_u64() else { continue };
            let texture = held.texture(item["name"].as_str().unwrap_or_default())?;
            self.device.note_texture_binding(commands, texture);
            let resource = self.device.texture_resource_id(texture)?;
            commands.set_tile_texture(resource, index as u32)?;
        }
        for item in case["samplers"].as_array().into_iter().flatten().filter(tile) {
            let Some(index) = item["index"].as_u64() else { continue };
            let resource = held.sampler(item["name"].as_str().unwrap_or_default())?;
            commands.set_tile_sampler(&mut self.device.resolver(), resource, None, index as u32)?;
        }
        Ok(())
    }

    /// Dispatches the tiles that run before the draws, or after them.
    pub(crate) fn dispatch_tiles(
        &mut self,
        tiles: &[TileDispatch],
        before: bool,
        commands: &mut CommandBuffer,
    ) -> Result<()> {
        for tile in tiles.iter().filter(|t| (t.spec["before"] == true) == before) {
            commands.set_tile_pipeline_state(&tile.state, tile.target)?;
            for memory in tile.spec["threadgroup_memory"].as_array().into_iter().flatten() {
                commands.set_tile_threadgroup_memory(
                    memory["length"].as_u64().unwrap_or(16),
                    memory["offset"].as_u64().unwrap_or(0),
                    memory["index"].as_u64().unwrap_or(0) as u32,
                )?;
            }
            let at = |i: usize| tile.spec["threads"][i].as_u64().unwrap_or(1);
            commands.dispatch_threads_per_tile(
                &mut self.device.resolver(),
                Size {
                    width: at(0),
                    height: at(1),
                    depth: 1,
                },
            )?;
        }
        Ok(())
    }
}
