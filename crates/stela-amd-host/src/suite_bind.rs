//! How a case's buffers, textures and samplers reach its encoder, as the
//! Mac's `bind_resources` binds them: each at its index on the stages it
//! names, a buffer at its offset (or its bytes recorded into the command
//! when it is `inline`), a sampler with its binding's LOD clamps.
//!
//! The `bind` modes ("single", "ranges", "offsets", "legacy") differ only
//! in which Metal calls the Mac makes; the bindings they leave are the same.
use super::suite::{Held, Runner};
use serde_json::Value;
use stela_driver_gpu::metal::{
    command_buffer::CommandBuffer,
    error::{Error, Result},
    shader::Stage,
};

/// The stages a binding names: a compute encoder has one.
pub fn stages(item: &Value, compute: bool) -> Result<&'static [Stage]> {
    if compute {
        return Ok(&[Stage::Kernel]);
    }
    Ok(match item["stage"].as_str().unwrap_or("fragment") {
        "vertex" => &[Stage::Vertex],
        "fragment" => &[Stage::Fragment],
        "both" => &[Stage::Vertex, Stage::Fragment],
        "object" => &[Stage::Object],
        "mesh" => &[Stage::Mesh],
        // Tile resources bind with the tile dispatches (`suite_tile`).
        "tile" => &[],
        _ => return Err(Error::Unsupported("a binding stage")),
    })
}

/// Whether the runner binds on the stage `item` names.
pub fn stage_known(item: &Value) -> bool {
    matches!(
        item["stage"].as_str(),
        None | Some("vertex" | "fragment" | "both" | "object" | "mesh" | "tile")
    )
}

impl Runner<'_> {
    /// Binds `spec`'s resources to the open encoder, a compute one when
    /// `compute`.
    pub(crate) fn bind_resources(
        &mut self,
        spec: &Value,
        held: &Held,
        commands: &mut CommandBuffer,
        compute: bool,
    ) -> Result<()> {
        for item in spec["buffers"].as_array().into_iter().flatten() {
            // A buffer without an index is bound nowhere (an index buffer).
            let Some(index) = item["index"].as_u64() else {
                continue;
            };
            let index = index as u32;
            let name = item["name"].as_str().unwrap_or_default();
            let at = *held
                .named
                .get(name)
                .ok_or(Error::Invalid("a binding names no buffer"))?;
            let buffer = &held.buffers[at];
            let offset = item["offset"].as_u64().unwrap_or(0);
            let stride = item["attribute_stride"].as_u64();
            for &stage in stages(item, compute)? {
                if item["inline"] == true {
                    let length = match item["length"].as_u64() {
                        Some(length) if length > 0 => length.min(buffer.length),
                        _ => buffer.length,
                    };
                    let bytes = buffer.bytes(length).to_vec();
                    let mut resolver = self.device.resolver();
                    match (stage, stride) {
                        (Stage::Kernel, Some(stride)) => commands
                            .set_compute_bytes_with_stride(&mut resolver, &bytes, stride, index)?,
                        (Stage::Kernel, None) => {
                            commands.set_compute_bytes(&mut resolver, &bytes, index)?
                        }
                        (_, None) => commands.set_render_bytes(&mut resolver, stage, &bytes, index)?,
                        _ => return Err(Error::Unsupported("an inline buffer with a stride")),
                    }
                    continue;
                }
                let reference = self.device.buffer(buffer.id)?;
                match (stage, stride) {
                    (Stage::Kernel, Some(stride)) => commands
                        .set_compute_buffer_with_stride(Some(reference), offset, stride, index)?,
                    (Stage::Kernel, None) => {
                        commands.set_compute_buffer(Some(reference), offset, index)?
                    }
                    (Stage::Vertex, Some(stride)) => {
                        commands.set_vertex_buffer_with_stride(reference, offset, stride, index)?
                    }
                    (_, None) => commands.set_render_buffer(stage, Some(reference), offset, index)?,
                    _ => return Err(Error::Unsupported("a fragment buffer with a stride")),
                }
            }
        }
        for item in spec["textures"].as_array().into_iter().flatten() {
            let Some(index) = item["index"].as_u64() else {
                continue;
            };
            let texture = held.texture(item["name"].as_str().unwrap_or_default())?;
            self.device.note_texture_binding(commands, texture);
            let resource = self.device.texture_resource_id(texture)?;
            for &stage in stages(item, compute)? {
                if compute {
                    commands.set_compute_texture(resource, index as u32)?;
                } else {
                    commands.set_render_texture(stage, resource, index as u32)?;
                }
            }
        }
        for item in spec["samplers"].as_array().into_iter().flatten() {
            let Some(index) = item["index"].as_u64() else {
                continue;
            };
            let resource = held.sampler(item["name"].as_str().unwrap_or_default())?;
            // `bind_lod` replaces the state's LOD clamps for this binding.
            let clamp = item["bind_lod"].as_array().map(|clamp| {
                let at = |i: usize| clamp.get(i).and_then(Value::as_f64).unwrap_or(0.0) as f32;
                (at(0), at(1))
            });
            for &stage in stages(item, compute)? {
                let mut resolver = self.device.resolver();
                if compute {
                    commands.set_compute_sampler(&mut resolver, resource, clamp, index as u32)?;
                } else {
                    commands.set_render_sampler(
                        &mut resolver,
                        stage,
                        resource,
                        clamp,
                        index as u32,
                    )?;
                }
            }
        }
        if compute {
            for memory in spec["threadgroup_memory"].as_array().into_iter().flatten() {
                commands.set_threadgroup_memory_length(
                    memory["length"].as_u64().unwrap_or(16),
                    memory["index"].as_u64().unwrap_or(0) as u32,
                )?;
            }
        }
        Ok(())
    }
}
