//! The suite's render cases, as the Mac's runner encodes them
//! (`encode_render` in `runner.m`): color, depth and stencil attachments
//! that load or clear and store, a pipeline with blending, write masks and a
//! vertex descriptor, buffers, textures and samplers per stage, the
//! encoder's depth, stencil and rasterizer state, then one draw or several,
//! indexed or not; each output read back with tight rows.
use super::suite::{BASIC_OUTPUT, Held, Runner, named, unmet};
use serde_json::{Map, Value};
use std::{path::Path, time::Instant};
use stela_driver_gpu::metal::{
    abi::{
        BlendFactor, BlendOperation, ColorWriteMask, CompareFunction, CullMode, IndexType,
        LoadAction, MultisampleDepthResolveFilter, MultisampleStencilResolveFilter,
        PrimitiveTopologyClass, PrimitiveType, StencilOperation, StoreAction, StoreActionOptions,
        VisibilityResultType, Winding,
    },
    command_buffer::render::{ScissorRect, Viewport},
    device::{
        TextureId,
        pipelines::{LinkedFunctions, PipelineArchives, RenderPipelineRequest},
        queue::{AttachmentRequest, RenderPassRequest},
    },
    error::{Error, Result},
    pipeline::{
        ColorAttachmentDescriptor, DepthStencilDescriptor, MAX_COLOR_ATTACHMENTS, RenderTargets,
        StencilDescriptor, depth_stencil,
    },
    shader::Stage,
    vertex::{Attribute, Layout, StepFunction, VertexDescriptor, VertexFormat},
};

const CASE: [&str; 14] = [
    "buffers",
    "constants",
    "fragment",
    "informational",
    "kind",
    "library",
    "name",
    "note",
    "outputs",
    "render",
    "requires",
    "samplers",
    "textures",
    "vertex",
];
const RENDER: [&str; 21] = [
    "base_instance",
    "blend_color",
    "colors",
    "cull",
    "depth",
    "depth_bias",
    "draws",
    "index_buffer",
    "index_offset",
    "index_type",
    "instance_count",
    "primitive",
    "rasterize",
    "scissor",
    "stencil",
    "target_size",
    "vertex_count",
    "vertex_descriptor",
    "vertex_start",
    "viewport",
    "winding",
];
const COLOR: [&str; 4] = ["blend", "clear", "texture", "write_mask"];
const DEPTH: [&str; 4] = ["clear", "compare", "texture", "write"];
const STENCIL: [&str; 12] = [
    "back",
    "clear",
    "compare",
    "depth_fail",
    "fail",
    "front",
    "pass",
    "read_mask",
    "reference",
    "reference_back",
    "texture",
    "write_mask",
];
const BUFFER: [&str; 7] = [
    "file",
    "index",
    "length",
    "name",
    "stage",
    "storage",
    "write_combined",
];

fn only(value: &Value, keys: &[&str]) -> bool {
    value
        .as_object()
        .is_some_and(|object| object.keys().all(|key| keys.contains(&key.as_str())))
}

fn all(value: &Value, keys: &[&str]) -> bool {
    value.is_null()
        || value
            .as_array()
            .is_some_and(|items| items.iter().all(|item| only(item, keys)))
}

/// Whether this runner reproduces the render case exactly.
pub fn basic(case: &Value) -> bool {
    let render = &case["render"];
    let stage_ok = |items: &Value| {
        items
            .as_array()
            .into_iter()
            .flatten()
            .all(|item| matches!(item["stage"].as_str(), None | Some("vertex" | "fragment")))
    };
    let mut texture_keys = super::suite_resources::TEXTURE.to_vec();
    texture_keys.push("stage");
    let mut sampler_keys = super::suite_resources::SAMPLER.to_vec();
    sampler_keys.push("stage");
    case["kind"] == "render"
        && only(case, &CASE)
        && only(render, &RENDER)
        && all(&render["colors"], &COLOR)
        && (render["depth"].is_null() || only(&render["depth"], &DEPTH))
        && (render["stencil"].is_null() || only(&render["stencil"], &STENCIL))
        && all(
            &render["draws"],
            &["instance_count", "vertex_count", "vertex_start"],
        )
        && all(&case["buffers"], &BUFFER)
        && stage_ok(&case["buffers"])
        && all(&case["textures"], &texture_keys)
        && case["textures"].as_array().into_iter().flatten().all(|t| {
            super::suite_resources::texture_type(t["type"].as_str().unwrap_or("2d")).is_some()
        })
        && stage_ok(&case["textures"])
        && all(&case["samplers"], &sampler_keys)
        && stage_ok(&case["samplers"])
        && all(&case["outputs"], &BASIC_OUTPUT)
}

fn stage(item: &Value) -> Stage {
    match item["stage"].as_str() {
        Some("vertex") => Stage::Vertex,
        _ => Stage::Fragment,
    }
}

fn compare(item: &Value, default: &str) -> Result<CompareFunction> {
    named(
        CompareFunction::ALL,
        item["compare"].as_str().unwrap_or(default),
    )
    .ok_or(Error::Unsupported("a compare function"))
}

fn stencil_face(face: &Value) -> Result<StencilDescriptor> {
    let operation = |key: &str| {
        named(StencilOperation::ALL, face[key].as_str().unwrap_or("keep"))
            .ok_or(Error::Unsupported("a stencil operation"))
    };
    Ok(StencilDescriptor {
        stencil_compare_function: compare(face, "always")?,
        stencil_failure_operation: operation("fail")?,
        depth_failure_operation: operation("depth_fail")?,
        depth_stencil_pass_operation: operation("pass")?,
        read_mask: face["read_mask"].as_u64().unwrap_or(0xff) as u32,
        write_mask: face["write_mask"].as_u64().unwrap_or(0xff) as u32,
    })
}

fn vertex_descriptor(spec: &Value) -> Result<VertexDescriptor> {
    let number = |item: &Value, key: &str, default: u64| item[key].as_u64().unwrap_or(default);
    let mut attributes = Vec::new();
    for item in spec["attributes"].as_array().into_iter().flatten() {
        attributes.push(Attribute {
            index: number(item, "index", 0) as u32,
            format: named(
                VertexFormat::ALL,
                item["format"].as_str().unwrap_or_default(),
            )
            .ok_or(Error::Unsupported("a vertex format"))?,
            offset: number(item, "offset", 0),
            buffer_index: number(item, "buffer", 0) as u32,
        });
    }
    let mut layouts = Vec::new();
    for item in spec["layouts"].as_array().into_iter().flatten() {
        let step = item["step"].as_str().unwrap_or("vertex");
        let step_function = match step {
            "instance" => StepFunction::PerInstance,
            "constant" => StepFunction::Constant,
            "vertex" => StepFunction::PerVertex,
            _ => return Err(Error::Unsupported("a vertex step function")),
        };
        if item["stride"].is_string() {
            return Err(Error::Unsupported("a dynamic vertex stride"));
        }
        layouts.push(Layout {
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
    Ok(VertexDescriptor {
        attributes,
        layouts,
    })
}

fn attachment<Clear>(
    texture: TextureId,
    clear: Option<Clear>,
    zero: Clear,
) -> AttachmentRequest<Clear> {
    AttachmentRequest {
        texture,
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
        clear: clear.unwrap_or(zero),
        surface: None,
    }
}

impl Runner<'_> {
    /// Runs one render case; the GPU's milliseconds.
    pub(crate) fn render_case(&mut self, case: &Value, out: &Path) -> Result<f64> {
        if unmet(self.device.capabilities(), &case["requires"]).is_some() {
            return Err(Error::Unsupported("a family or feature the device lacks"));
        }
        let mut held = Held::default();
        let result = self.render_inner(case, out, &mut held);
        self.release(held);
        result
    }

    fn texture_named(held: &Held, name: &str) -> Result<TextureId> {
        held.textures
            .iter()
            .find(|(t, _)| t == name)
            .map(|(_, texture)| texture.id)
            .ok_or(Error::Invalid("an attachment names no texture"))
    }

    fn render_inner(&mut self, case: &Value, out: &Path, held: &mut Held) -> Result<f64> {
        let render = &case["render"];
        let empty = Map::new();
        let mut commands = self.device.new_command_buffer();
        self.make_buffers(&case["buffers"], held)?;
        held.textures = self.make_textures(&case["textures"], &mut commands, &mut held.buffers)?;
        let mut samplers = Vec::new();
        for spec in case["samplers"].as_array().into_iter().flatten() {
            let (id, resource) = self.make_sampler(spec)?;
            held.samplers.push(id);
            samplers.push((
                stage(spec),
                resource,
                spec["index"].as_u64().unwrap_or(0) as u32,
            ));
        }

        // The pipeline's targets and the pass's attachments.
        let library = self.library(case["library"].as_str().unwrap_or_default())?;
        let constants = super::suite_constants::values(&case["constants"])?;
        let vertex = self.device.new_function(
            library,
            case["vertex"].as_str().unwrap_or_default(),
            Some(&constants),
        )?;
        let fragment = match case["fragment"].as_str() {
            Some(name) => Some(self.device.new_function(library, name, Some(&constants))?),
            None => None,
        };
        let mut targets = RenderTargets::default();
        let mut colors = [None; MAX_COLOR_ATTACHMENTS];
        let specs = render["colors"].as_array().map_or(&[][..], Vec::as_slice);
        if specs.len() > MAX_COLOR_ATTACHMENTS {
            return Err(Error::Unsupported("more color attachments than Metal has"));
        }
        for (index, spec) in specs.iter().enumerate() {
            let texture = Self::texture_named(held, spec["texture"].as_str().unwrap_or_default())?;
            let format = self.device.texture_descriptor(texture)?.pixel_format;
            let mut descriptor = ColorAttachmentDescriptor {
                pixel_format: Some(format),
                ..ColorAttachmentDescriptor::default()
            };
            if let Some(blend) = spec["blend"].as_object() {
                let factor = |key: &str, default: &str| {
                    let name = blend.get(key).and_then(Value::as_str).unwrap_or(default);
                    named(BlendFactor::ALL, name).ok_or(Error::Unsupported("a blend factor"))
                };
                let operation = |key: &str| {
                    let name = blend.get(key).and_then(Value::as_str).unwrap_or("add");
                    named(BlendOperation::ALL, name).ok_or(Error::Unsupported("a blend operation"))
                };
                descriptor.blending_enabled = true;
                descriptor.rgb_blend_operation = operation("rgb")?;
                descriptor.alpha_blend_operation = operation("alpha")?;
                descriptor.source_rgb_blend_factor = factor("src_rgb", "one")?;
                descriptor.destination_rgb_blend_factor = factor("dst_rgb", "zero")?;
                descriptor.source_alpha_blend_factor = factor("src_alpha", "one")?;
                descriptor.destination_alpha_blend_factor = factor("dst_alpha", "zero")?;
            }
            if let Some(mask) = spec["write_mask"].as_str() {
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
            let clear = spec["clear"].as_array().map(|c| {
                core::array::from_fn(|channel| {
                    c.get(channel).and_then(Value::as_f64).unwrap_or(0.0)
                })
            });
            colors[index] = Some(attachment(texture, clear, [0.0; 4]));
        }
        let mut depth_stencil_descriptor = DepthStencilDescriptor::default();
        let mut depth_attachment = None;
        let depth = &render["depth"];
        if depth.is_object() {
            let texture = Self::texture_named(held, depth["texture"].as_str().unwrap_or_default())?;
            targets.depth_attachment_pixel_format =
                Some(self.device.texture_descriptor(texture)?.pixel_format);
            depth_attachment = Some(attachment(texture, depth["clear"].as_f64(), 0.0));
            depth_stencil_descriptor.depth_compare_function = compare(depth, "always")?;
            depth_stencil_descriptor.depth_write_enabled = depth["write"].as_bool().unwrap_or(true);
        }
        let mut stencil_attachment = None;
        let mut references = None;
        let stencil = &render["stencil"];
        if stencil.is_object() {
            let texture =
                Self::texture_named(held, stencil["texture"].as_str().unwrap_or_default())?;
            targets.stencil_attachment_pixel_format =
                Some(self.device.texture_descriptor(texture)?.pixel_format);
            let clear = stencil["clear"].as_u64().map(|clear| clear as u32);
            stencil_attachment = Some(attachment(texture, clear, 0));
            let face = |name: &str| {
                if stencil[name].is_object() {
                    stencil_face(&stencil[name])
                } else {
                    stencil_face(stencil)
                }
            };
            depth_stencil_descriptor.front_face_stencil = face("front")?;
            depth_stencil_descriptor.back_face_stencil = face("back")?;
            let front = stencil["reference"].as_u64().unwrap_or(0) as u32;
            let back = stencil["reference_back"]
                .as_u64()
                .map_or(front, |back| back as u32);
            references = Some((front, back));
        }
        if render["rasterize"] == false {
            targets.rasterization_enabled = false;
        }
        let vertex_descriptor = if render["vertex_descriptor"].is_object() {
            Some(vertex_descriptor(&render["vertex_descriptor"])?)
        } else {
            None
        };
        let pipeline = self.device.new_render_pipeline(&RenderPipelineRequest {
            vertex_function: vertex,
            fragment_function: fragment,
            vertex_descriptor,
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
        let size = |at: usize| render["target_size"][at].as_u64().unwrap_or(0);
        let pass = RenderPassRequest {
            color_attachments: colors,
            depth_attachment,
            depth_resolve_filter: MultisampleDepthResolveFilter::Sample0,
            stencil_attachment,
            stencil_resolve_filter: MultisampleStencilResolveFilter::Sample0,
            visibility_result_buffer: None,
            visibility_result_type: VisibilityResultType::Reset,
            render_target_array_length: 0,
            render_target_width: size(0),
            render_target_height: size(1),
            default_raster_sample_count: u64::from(render["target_size"].is_array()),
            tile_memory: Default::default(),
            sample_positions: Vec::new(),
            rasterization_rate_map: false,
            counter_samples: Vec::new(),
        };

        // The encoder's state, the bindings, the draws.
        self.device.begin_render(&mut commands, &pass)?;
        commands.set_render_pipeline_state(&state)?;
        commands.set_depth_stencil_state(Some(&depth_stencil(&depth_stencil_descriptor)))?;
        if let Some((front, back)) = references {
            commands.set_stencil_reference_values(front, back)?;
        }
        if let Some(bias) = render["depth_bias"].as_array() {
            let value = |at: usize| bias.get(at).and_then(Value::as_f64).unwrap_or(0.0) as f32;
            commands.set_depth_bias(value(0), value(1), value(2))?;
        }
        if let Some(color) = render["blend_color"].as_array() {
            let value = |at: usize| color.get(at).and_then(Value::as_f64).unwrap_or(0.0) as f32;
            commands.set_blend_color([value(0), value(1), value(2), value(3)])?;
        }
        let cull = match render["cull"].as_str().unwrap_or("none") {
            "front" => CullMode::Front,
            "back" => CullMode::Back,
            _ => CullMode::None,
        };
        commands.set_cull_mode(cull)?;
        let winding = if render["winding"] == "cw" {
            Winding::Clockwise
        } else {
            Winding::CounterClockwise
        };
        commands.set_front_facing_winding(winding)?;
        if let Some(v) = render["viewport"].as_array() {
            let value = |at: usize| v.get(at).and_then(Value::as_f64).unwrap_or(0.0);
            commands.set_viewports(&[Viewport {
                origin_x: value(0),
                origin_y: value(1),
                width: value(2),
                height: value(3),
                znear: 0.0,
                zfar: 1.0,
            }])?;
        }
        if let Some(s) = render["scissor"].as_array() {
            let value = |at: usize| s.get(at).and_then(Value::as_u64).unwrap_or(0);
            commands.set_scissor_rects(&[ScissorRect {
                x: value(0),
                y: value(1),
                width: value(2),
                height: value(3),
            }])?;
        }
        for spec in case["buffers"].as_array().into_iter().flatten() {
            let name = spec["name"].as_str().unwrap_or_default();
            let buffer = &held.buffers[held.named[name]];
            // A buffer without an index is bound nowhere (an index buffer).
            let Some(index) = spec["index"].as_u64() else {
                continue;
            };
            let index = index as u32;
            let reference = self.device.buffer(buffer.id)?;
            commands.set_render_buffer(stage(spec), Some(reference), 0, index)?;
        }
        for spec in case["textures"].as_array().into_iter().flatten() {
            let Some(index) = spec["index"].as_u64() else {
                continue;
            };
            let texture = Self::texture_named(held, spec["name"].as_str().unwrap_or_default())?;
            self.device.note_texture_binding(&mut commands, texture);
            let resource = self.device.texture_resource_id(texture)?;
            commands.set_render_texture(stage(spec), resource, index as u32)?;
        }
        for (stage, resource, index) in samplers {
            commands.set_render_sampler(
                &mut self.device.resolver(),
                stage,
                resource,
                None,
                index,
            )?;
        }
        let primitive = named(
            PrimitiveType::ALL,
            render["primitive"].as_str().unwrap_or("triangle"),
        )
        .ok_or(Error::Unsupported("a primitive type"))?;
        let draws: Vec<&Map<String, Value>> = match render["draws"].as_array() {
            Some(draws) => draws.iter().filter_map(Value::as_object).collect(),
            None => vec![render.as_object().unwrap_or(&empty)],
        };
        for draw in draws {
            let count = |key: &str, default: u64| {
                draw.get(key)
                    .and_then(Value::as_u64)
                    .or_else(|| render[key].as_u64())
                    .unwrap_or(default) as u32
            };
            if let Some(name) = render["index_buffer"].as_str() {
                let at = *held
                    .named
                    .get(name)
                    .ok_or(Error::Invalid("the index buffer is no buffer"))?;
                let index_type = if render["index_type"] == "uint32" {
                    IndexType::UInt32
                } else {
                    IndexType::UInt16
                };
                let index_buffer = self.device.buffer(held.buffers[at].id)?;
                commands.draw_indexed_primitives(
                    &mut self.device.resolver(),
                    primitive,
                    count("vertex_count", 3),
                    index_type,
                    index_buffer,
                    render["index_offset"].as_u64().unwrap_or(0),
                    count("instance_count", 1),
                    0,
                    count("base_instance", 0),
                )?;
            } else {
                commands.draw_primitives(
                    &mut self.device.resolver(),
                    primitive,
                    count("vertex_start", 0),
                    count("vertex_count", 3),
                    count("instance_count", 1),
                    count("base_instance", 0),
                )?;
            }
        }
        commands.end_encoding(&mut self.device.resolver())?;
        let readbacks = self.read_back_outputs(case, held, &mut commands)?;
        let started = Instant::now();
        let serial = self.device.commit(commands)?;
        super::device::wait(self.device, serial)?;
        let gpu_ms = started.elapsed().as_secs_f64() * 1e3;
        self.store_outputs(case, held, &readbacks, out)?;
        Ok(gpu_ms)
    }
}
