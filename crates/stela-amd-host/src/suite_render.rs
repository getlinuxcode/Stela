//! The suite's render cases, as the Mac's runner encodes them
//! (`encode_render`, `make_texture`, `store_outputs` in `runner.m`): 2D
//! textures loaded from their files, a pass over color attachments that
//! load or clear and store, a pipeline blending as the case names, buffers
//! per stage, one draw, then each output read back with tight rows.
use super::{
    device::shared_buffer,
    suite::{Runner, named},
};
use serde_json::Value;
use std::{collections::BTreeMap, fs, path::Path, time::Instant};
use stela_driver_gpu::metal::{
    abi::{
        BlendFactor, BlendOperation, BlitOption, ColorWriteMask, CpuCacheMode, HazardTrackingMode,
        LoadAction, MultisampleDepthResolveFilter, MultisampleStencilResolveFilter,
        PrimitiveTopologyClass, PrimitiveType, ResourceOptions, StorageMode, StoreAction,
        StoreActionOptions, SwizzleChannels, TextureType, TextureUsage, VisibilityResultType,
    },
    command_buffer::blit::{BlitTexture, BufferLayout, Region},
    device::{
        BufferId, TextureId,
        pipelines::{LinkedFunctions, PipelineArchives, RenderPipelineRequest},
        queue::{AttachmentRequest, RenderPassRequest},
    },
    error::{Error, Result},
    pipeline::{ColorAttachmentDescriptor, MAX_COLOR_ATTACHMENTS, RenderTargets},
    pixel_format::PixelFormat,
    resource::TextureDescriptor,
    shader::Stage,
};

const CASE: [&str; 10] = [
    "buffers", "fragment", "kind", "library", "name", "outputs", "render", "samplers", "textures",
    "vertex",
];
const RENDER: [&str; 6] = [
    "blend_color",
    "colors",
    "instance_count",
    "primitive",
    "vertex_count",
    "vertex_start",
];
const TEXTURE: [&str; 8] = [
    "depth", "file", "format", "height", "name", "type", "usage", "width",
];
const COLOR: [&str; 4] = ["blend", "clear", "texture", "write_mask"];
const BUFFER: [&str; 5] = ["file", "index", "length", "name", "stage"];
const OUTPUT: [&str; 6] = [
    "dtype",
    "expect",
    "expect_tolerance",
    "length",
    "name",
    "tolerance",
];

fn only(value: &Value, keys: &[&str]) -> bool {
    value
        .as_object()
        .is_some_and(|object| object.keys().all(|key| keys.contains(&key.as_str())))
}

fn all(value: &Value, keys: &[&str]) -> bool {
    value
        .as_array()
        .is_some_and(|items| items.iter().all(|item| only(item, keys)))
}

/// Whether this runner reproduces the render case exactly.
pub fn basic(case: &Value) -> bool {
    case["kind"] == "render"
        && only(case, &CASE)
        && case["samplers"].as_array().is_some_and(Vec::is_empty)
        && only(&case["render"], &RENDER)
        && all(&case["render"]["colors"], &COLOR)
        && all(&case["textures"], &TEXTURE)
        && case["textures"].as_array().is_some_and(|textures| {
            textures
                .iter()
                .all(|t| t["type"].as_str().unwrap_or("2d") == "2d")
        })
        && all(&case["buffers"], &BUFFER)
        && all(&case["outputs"], &OUTPUT)
}

/// A texture the case made, and its descriptor.
struct Texture {
    id: TextureId,
    descriptor: TextureDescriptor,
}

impl Texture {
    /// Bytes of one tight row, and of the level.
    fn layout(&self) -> (u64, u64) {
        let block = self.descriptor.pixel_format.describe().block;
        let across = self.descriptor.width.div_ceil(u64::from(block.width));
        let down = self.descriptor.height.div_ceil(u64::from(block.height));
        let row = across * u64::from(block.bytes);
        (row, row * down)
    }

    fn region(&self) -> Region {
        Region {
            slice: 0,
            level: 0,
            origin: [0; 3],
            size: [self.descriptor.width, self.descriptor.height, 1],
        }
    }
}

/// A shared buffer the case uses, and its mapping.
struct Shared {
    id: BufferId,
    pointer: core::ptr::NonNull<u8>,
    length: u64,
}

impl Runner<'_> {
    /// A shared buffer of `length` bytes holding `data`.
    fn shared(&mut self, data: &[u8], length: u64) -> Result<Shared> {
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

    fn input(&self, spec: &Value) -> Result<Vec<u8>> {
        match spec["file"].as_str() {
            Some(file) => fs::read(self.plan_dir.join(file))
                .map_err(|_| Error::Invalid("an input file is missing")),
            None => Ok(Vec::new()),
        }
    }

    /// Runs one render case; the GPU's milliseconds.
    pub(crate) fn render_case(&mut self, case: &Value, out: &Path) -> Result<f64> {
        let mut buffers: Vec<Shared> = Vec::new();
        let mut textures: BTreeMap<String, Texture> = BTreeMap::new();
        let result = self.render_inner(case, out, &mut buffers, &mut textures);
        for buffer in buffers {
            let _ = self.device.destroy_buffer(buffer.id);
        }
        for texture in textures.into_values() {
            let _ = self.device.destroy_texture(texture.id);
        }
        let _ = self.device.poll();
        while self.device.retired().is_some() {}
        result
    }

    fn render_inner(
        &mut self,
        case: &Value,
        out: &Path,
        buffers: &mut Vec<Shared>,
        textures: &mut BTreeMap<String, Texture>,
    ) -> Result<f64> {
        let render = &case["render"];
        let mut commands = self.device.new_command_buffer();

        // Textures, private, loaded through a staging buffer and a blit.
        let mut uploads = Vec::new();
        for spec in case["textures"].as_array().into_iter().flatten() {
            let format = spec["format"].as_str().unwrap_or_default();
            let pixel_format = named(PixelFormat::ALL, format).ok_or(Error::Unsupported(
                "a pixel format this runner does not name",
            ))?;
            let mut usage = TextureUsage::EMPTY;
            for item in spec["usage"].as_array().into_iter().flatten() {
                usage |= match item.as_str() {
                    Some("read") => TextureUsage::SHADER_READ,
                    Some("write") => TextureUsage::SHADER_WRITE,
                    Some("render") => TextureUsage::RENDER_TARGET,
                    _ => return Err(Error::Unsupported("a texture usage")),
                };
            }
            if spec["usage"].is_null() {
                usage = TextureUsage::SHADER_READ;
            }
            let descriptor = TextureDescriptor {
                texture_type: TextureType::D2,
                pixel_format,
                width: spec["width"].as_u64().unwrap_or(1),
                height: spec["height"].as_u64().unwrap_or(1),
                depth: spec["depth"].as_u64().unwrap_or(1),
                mipmap_level_count: 1,
                sample_count: 1,
                array_length: 1,
                options: ResourceOptions {
                    cpu_cache_mode: CpuCacheMode::DefaultCache,
                    storage_mode: StorageMode::Private,
                    hazard_tracking_mode: HazardTrackingMode::Default,
                },
                usage,
                swizzle: SwizzleChannels::IDENTITY,
            };
            let id = self.device.new_texture(&descriptor)?;
            let texture = Texture { id, descriptor };
            let name = spec["name"].as_str().unwrap_or_default().to_owned();
            if spec["file"].is_string() {
                let data = self.input(spec)?;
                let (_, total) = texture.layout();
                if (data.len() as u64) < total {
                    return Err(Error::Invalid("a texture input is short"));
                }
                let staging = self.shared(&data[..total as usize], total)?;
                uploads.push((name.clone(), staging.id));
                buffers.push(staging);
            }
            textures.insert(name, texture);
        }
        if !uploads.is_empty() {
            commands.begin_blit()?;
            for (name, staging) in &uploads {
                let texture = &textures[name];
                let (row, total) = texture.layout();
                let source = BufferLayout {
                    buffer: self.device.buffer(*staging)?,
                    offset: 0,
                    bytes_per_row: row,
                    bytes_per_image: total,
                };
                let target = BlitTexture {
                    image: self.device.texture_image(texture.id)?,
                    descriptor: &texture.descriptor,
                    storage: self.device.texel_storage(texture.id, 0, 0)?,
                    blocks: self.device.texture_blocks(texture.id)?,
                };
                commands.copy_buffer_to_texture(
                    &mut self.device.resolver(),
                    source,
                    target,
                    texture.region(),
                    BlitOption::EMPTY,
                )?;
            }
            commands.end_encoding(&mut self.device.resolver())?;
        }

        // The pipeline and the pass.
        let library = self.library(case["library"].as_str().unwrap_or_default())?;
        let vertex = self.device.new_function(
            library,
            case["vertex"].as_str().unwrap_or_default(),
            Some(&[]),
        )?;
        let fragment = match case["fragment"].as_str() {
            Some(name) => Some(self.device.new_function(library, name, Some(&[]))?),
            None => None,
        };
        let mut targets = RenderTargets::default();
        let mut colors = [None; MAX_COLOR_ATTACHMENTS];
        let attachments = render["colors"].as_array().map_or(&[][..], Vec::as_slice);
        if attachments.len() > MAX_COLOR_ATTACHMENTS {
            return Err(Error::Unsupported("more color attachments than Metal has"));
        }
        for (index, attachment) in attachments.iter().enumerate() {
            let name = attachment["texture"].as_str().unwrap_or_default();
            let texture = textures
                .get(name)
                .ok_or(Error::Invalid("a color attachment names no texture"))?;
            let mut descriptor = ColorAttachmentDescriptor {
                pixel_format: Some(texture.descriptor.pixel_format),
                ..ColorAttachmentDescriptor::default()
            };
            if let Some(blend) = attachment["blend"].as_object() {
                let factor = |key: &str, default: &str| {
                    named(
                        BlendFactor::ALL,
                        blend.get(key).and_then(Value::as_str).unwrap_or(default),
                    )
                    .ok_or(Error::Unsupported("a blend factor"))
                };
                let operation = |key: &str| {
                    named(
                        BlendOperation::ALL,
                        blend.get(key).and_then(Value::as_str).unwrap_or("add"),
                    )
                    .ok_or(Error::Unsupported("a blend operation"))
                };
                descriptor.blending_enabled = true;
                descriptor.rgb_blend_operation = operation("rgb")?;
                descriptor.alpha_blend_operation = operation("alpha")?;
                descriptor.source_rgb_blend_factor = factor("src_rgb", "one")?;
                descriptor.destination_rgb_blend_factor = factor("dst_rgb", "zero")?;
                descriptor.source_alpha_blend_factor = factor("src_alpha", "one")?;
                descriptor.destination_alpha_blend_factor = factor("dst_alpha", "zero")?;
            }
            if let Some(mask) = attachment["write_mask"].as_str() {
                let mut bits = ColorWriteMask::EMPTY;
                for (letter, bit) in [
                    ('r', ColorWriteMask::RED),
                    ('g', ColorWriteMask::GREEN),
                    ('b', ColorWriteMask::BLUE),
                    ('a', ColorWriteMask::ALPHA),
                ] {
                    if mask.contains(letter) {
                        bits |= bit;
                    }
                }
                descriptor.write_mask = bits;
            }
            targets.color_attachments[index] = descriptor;
            let clear = attachment["clear"].as_array();
            colors[index] = Some(AttachmentRequest {
                texture: texture.id,
                level: 0,
                slice: 0,
                depth_plane: 0,
                resolve: None,
                load_action: if clear.is_some() {
                    LoadAction::Clear
                } else {
                    LoadAction::Load
                },
                store_action: StoreAction::Store,
                store_action_options: StoreActionOptions::EMPTY,
                clear: core::array::from_fn(|channel| {
                    clear.and_then(|c| c.get(channel)?.as_f64()).unwrap_or(0.0)
                }),
                surface: None,
            });
        }
        let pipeline = self.device.new_render_pipeline(&RenderPipelineRequest {
            vertex_function: vertex,
            fragment_function: fragment,
            vertex_descriptor: None,
            targets,
            input_primitive_topology: PrimitiveTopologyClass::Unspecified,
            max_vertex_amplification_count: 1,
            tessellation: Default::default(),
            vertex_linked_functions: LinkedFunctions::default(),
            fragment_linked_functions: LinkedFunctions::default(),
            support_indirect_command_buffers: false,
            archives: PipelineArchives::default(),
        })?;
        let state = self.device.render_pipeline_state(pipeline)?.clone();
        let pass = RenderPassRequest {
            color_attachments: colors,
            depth_attachment: None,
            depth_resolve_filter: MultisampleDepthResolveFilter::Sample0,
            stencil_attachment: None,
            stencil_resolve_filter: MultisampleStencilResolveFilter::Sample0,
            visibility_result_buffer: None,
            visibility_result_type: VisibilityResultType::Reset,
            render_target_array_length: 0,
            render_target_width: 0,
            render_target_height: 0,
            default_raster_sample_count: 0,
            tile_memory: Default::default(),
            sample_positions: Vec::new(),
            rasterization_rate_map: false,
            counter_samples: Vec::new(),
        };

        // Buffers, then the draw.
        let mut named_buffers = BTreeMap::new();
        let mut bindings = Vec::new();
        for spec in case["buffers"].as_array().into_iter().flatten() {
            let data = self.input(spec)?;
            let shared = self.shared(&data, spec["length"].as_u64().unwrap_or(16))?;
            let stage = match spec["stage"].as_str().unwrap_or("vertex") {
                "vertex" => Stage::Vertex,
                "fragment" => Stage::Fragment,
                _ => return Err(Error::Unsupported("a buffer stage")),
            };
            let index = spec["index"].as_u64().unwrap_or(0) as u32;
            bindings.push((stage, shared.id, index));
            named_buffers.insert(
                spec["name"].as_str().unwrap_or_default().to_owned(),
                buffers.len(),
            );
            buffers.push(shared);
        }
        self.device.begin_render(&mut commands, &pass)?;
        commands.set_render_pipeline_state(&state)?;
        for (stage, id, index) in bindings {
            commands.set_render_buffer(stage, Some(self.device.buffer(id)?), 0, index)?;
        }
        if let Some(color) = render["blend_color"].as_array() {
            let channel = |at: usize| color.get(at).and_then(Value::as_f64).unwrap_or(0.0) as f32;
            commands.set_blend_color([channel(0), channel(1), channel(2), channel(3)])?;
        }
        let primitive = named(
            PrimitiveType::ALL,
            render["primitive"].as_str().unwrap_or("triangle"),
        )
        .ok_or(Error::Unsupported("a primitive type"))?;
        let count = |key: &str, default: u64| render[key].as_u64().unwrap_or(default) as u32;
        commands.draw_primitives(
            &mut self.device.resolver(),
            primitive,
            count("vertex_start", 0),
            count("vertex_count", 3),
            count("instance_count", 1),
            0,
        )?;
        commands.end_encoding(&mut self.device.resolver())?;

        // Texture outputs come back through shared buffers.
        let mut readbacks = Vec::new();
        for output in case["outputs"].as_array().into_iter().flatten() {
            let name = output["name"].as_str().unwrap_or_default();
            if let Some(texture) = textures.get(name) {
                let (_, total) = texture.layout();
                let readback = self.shared(&[], total)?;
                readbacks.push((name.to_owned(), buffers.len(), total));
                buffers.push(readback);
            }
        }
        if !readbacks.is_empty() {
            commands.begin_blit()?;
            for (name, at, total) in &readbacks {
                let texture = &textures[name];
                let (row, _) = texture.layout();
                let source = BlitTexture {
                    image: self.device.texture_image(texture.id)?,
                    descriptor: &texture.descriptor,
                    storage: self.device.texel_storage(texture.id, 0, 0)?,
                    blocks: self.device.texture_blocks(texture.id)?,
                };
                let layout = BufferLayout {
                    buffer: self.device.buffer(buffers[*at].id)?,
                    offset: 0,
                    bytes_per_row: row,
                    bytes_per_image: *total,
                };
                commands.copy_texture_to_buffer(
                    &mut self.device.resolver(),
                    source,
                    texture.region(),
                    layout,
                    BlitOption::EMPTY,
                )?;
            }
            commands.end_encoding(&mut self.device.resolver())?;
        }
        let started = Instant::now();
        let serial = self.device.commit(commands)?;
        super::device::wait(self.device, serial)?;
        let gpu_ms = started.elapsed().as_secs_f64() * 1e3;

        for output in case["outputs"].as_array().into_iter().flatten() {
            let name = output["name"].as_str().unwrap_or_default();
            let (at, length) = if let Some((_, at, total)) = readbacks.iter().find(|r| r.0 == name)
            {
                (*at, *total)
            } else if let Some(&at) = named_buffers.get(name) {
                let length = buffers[at].length;
                (
                    at,
                    output["length"]
                        .as_u64()
                        .map_or(length, |declared| declared.min(length)),
                )
            } else {
                return Err(Error::Invalid(
                    "an output is neither a buffer nor a texture",
                ));
            };
            // SAFETY: the GPU finished writing the mapping.
            let bytes = unsafe {
                core::slice::from_raw_parts(buffers[at].pointer.as_ptr(), length as usize)
            };
            fs::write(out.join(format!("{name}.bin")), bytes)
                .map_err(|_| Error::Invalid("cannot write an output"))?;
        }
        Ok(gpu_ms)
    }
}
