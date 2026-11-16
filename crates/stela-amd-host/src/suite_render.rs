//! The suite's render cases, as the Mac's runner encodes them
//! (`encode_render` in `runner.m`): color, depth and stencil attachments
//! that load or clear and store, a pipeline with blending, write masks and a
//! vertex descriptor, buffers, textures and samplers per stage, the
//! encoder's depth, stencil and rasterizer state, then one draw or several,
//! indexed or not; each output read back with tight rows.
use super::suite::{BASIC_OUTPUT, Held, Runner, named};
use serde_json::{Map, Value};
use stela_driver_gpu::metal::{
    abi::{
        BlendFactor, BlendOperation, ColorWriteMask, CompareFunction, CullMode, DepthClipMode,
        IndexType, LoadAction, MultisampleDepthResolveFilter, MultisampleStencilResolveFilter,
        PrimitiveTopologyClass, PrimitiveType, StencilOperation, StoreAction, StoreActionOptions,
        TessellationControlPointIndexType, TessellationFactorFormat,
        TessellationFactorStepFunction, TessellationPartitionMode, TextureType, TriangleFillMode,
        VisibilityResultMode, VisibilityResultType, Winding,
    },
    command_buffer::{
        CommandBuffer,
        render::{PatchSource, ScissorRect, Viewport},
    },
    device::{
        TextureId,
        pipelines::{
            LinkedFunctions, MeshPipelineRequest, PipelineArchives, RenderPipelineRequest,
        },
        queue::{AttachmentRequest, RenderPassRequest},
    },
    error::{Error, Result},
    pipeline::{
        ColorAttachmentDescriptor, DepthStencilDescriptor, MAX_COLOR_ATTACHMENTS, RenderTargets,
        StageThreads, StencilDescriptor, TessellationDescriptor, depth_stencil,
    },
    vertex::{Attribute, Layout, STRIDE_DYNAMIC, StepFunction, VertexDescriptor, VertexFormat},
};

const CASE: [&str; 22] = [
    "uses",
    "pipeline_creation",
    "icbs",
    "use",
    "arguments",
    "heaps",
    "use_heaps",
    "bind",
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
const RENDER: [&str; 55] = [
    "amplification",
    "tiles",
    "tile_size",
    "tile_memory",
    "imageblock_sample_length",
    "vertex_function_groups",
    "fragment_function_groups",
    "vertex_visible_functions",
    "fragment_visible_functions",
    "vertex_private_functions",
    "fragment_private_functions",
    "vertex_binary_functions",
    "fragment_binary_functions",
    "vertex_tables",
    "fragment_tables",
    "vertex_call_stack_depth",
    "fragment_call_stack_depth",
    "handles_by_name",
    "call_stack_depth",
    "execute_icb",
    "supports_icb",
    "input_primitive",
    "mesh",
    "tessellation",
    "alpha_to_coverage",
    "alpha_to_one",
    "depth_clip",
    "fill",
    "indirect",
    "sample_positions",
    "viewports",
    "visibility",
    "base_vertex",
    "samples",
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
const COLOR: [&str; 12] = [
    "blend",
    "depth_plane",
    "level",
    "slice",
    "clear",
    "resolve",
    "resolve_depth_plane",
    "resolve_level",
    "resolve_slice",
    "store",
    "texture",
    "write_mask",
];
const DEPTH: [&str; 6] = [
    "clear",
    "compare",
    "resolve",
    "resolve_filter",
    "texture",
    "write",
];
const STENCIL: [&str; 14] = [
    "resolve",
    "resolve_filter",
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
const BUFFER: [&str; 16] = [
    "no_copy",
    "untracked",
    "alias_of",
    "heap",
    "heap_offset",
    "attribute_stride",
    "inline",
    "mutability",
    "offset",
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
            .all(super::suite_bind::stage_known)
    };
    let mut texture_keys = super::suite_resources::TEXTURE.to_vec();
    texture_keys.push("stage");
    let mut sampler_keys = super::suite_resources::SAMPLER.to_vec();
    sampler_keys.push("stage");
    case["kind"] == "render"
        && only(case, &CASE)
        && only(render, &RENDER)
        && all(&render["tiles"], &super::suite_tile::TILE)
        && super::suite_tables::supported(render, "vertex_")
        && super::suite_tables::supported(render, "fragment_")
        && all(&case["heaps"], &super::suite_heaps::HEAP)
        && super::suite_arguments::supported(case)
        && super::suite_icb::supported(case)
        && super::suite_icb::execution_supported(&render["execute_icb"])
        && (render["tessellation"].is_null() || only(&render["tessellation"], &TESSELLATION))
        && (render["mesh"].is_null() || only(&render["mesh"], &MESH))
        && all(&render["colors"], &COLOR)
        && (render["depth"].is_null() || only(&render["depth"], &DEPTH))
        && (render["stencil"].is_null() || only(&render["stencil"], &STENCIL))
        && all(
            &render["draws"],
            &[
                "instance_count",
                "vertex_count",
                "vertex_start",
                "visibility_mode",
                "visibility_offset",
            ],
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

/// Whether every output is one this runner writes.
pub fn all_outputs(outputs: &Value) -> bool {
    all(outputs, &BASIC_OUTPUT)
}

const TESSELLATION: [&str; 13] = [
    "control_point_index",
    "control_point_index_type",
    "control_points",
    "factor_scale",
    "factor_step",
    "factors",
    "indirect",
    "instance_count",
    "instance_stride",
    "max_factor",
    "partition",
    "patch_count",
    "patch_index",
];
const MESH: [&str; 11] = [
    "group_multiple_of_width",
    "indirect",
    "max_threadgroups",
    "mesh",
    "mesh_threads",
    "object",
    "object_threadgroup_memory",
    "object_threads",
    "payload_length",
    "threadgroups",
    "threads",
];

/// The pipeline's tessellation, as the runner's: half factors, integer
/// partitions unless named, per-instance factors when the draw has a stride.
fn tessellation(tess: &Value) -> Result<TessellationDescriptor> {
    let step = tess["factor_step"]
        .as_str()
        .unwrap_or(if tess["instance_stride"].is_null() {
            "per_patch"
        } else {
            "per_patch_and_per_instance"
        });
    Ok(TessellationDescriptor {
        partition_mode: named(
            TessellationPartitionMode::ALL,
            tess["partition"].as_str().unwrap_or("integer"),
        )
        .ok_or(Error::Unsupported("a partition mode"))?,
        max_tessellation_factor: tess["max_factor"].as_u64().unwrap_or(16),
        factor_scale_enabled: !tess["factor_scale"].is_null(),
        factor_format: TessellationFactorFormat::Half,
        control_point_index_type: if tess["control_point_index"].is_null() {
            TessellationControlPointIndexType::None
        } else if tess["control_point_index_type"] == "uint16" {
            TessellationControlPointIndexType::UInt16
        } else {
            TessellationControlPointIndexType::UInt32
        },
        factor_step_function: named(TessellationFactorStepFunction::ALL, step)
            .ok_or(Error::Unsupported("a factor step function"))?,
        output_winding_order: if tess["winding"] == "cw" {
            Winding::Clockwise
        } else {
            Winding::CounterClockwise
        },
    })
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
            "patch" => StepFunction::PerPatch,
            "patch_control_point" => StepFunction::PerPatchControlPoint,
            _ => return Err(Error::Unsupported("a vertex step function")),
        };
        // A dynamic stride is the buffer binding's attribute stride.
        let stride = match item["stride"].as_str() {
            Some("dynamic") => STRIDE_DYNAMIC,
            Some(_) => return Err(Error::Unsupported("a vertex stride")),
            None => number(item, "stride", 16),
        };
        layouts.push(Layout {
            buffer_index: number(item, "buffer", 0) as u32,
            stride,
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
    fn texture_named(held: &Held, name: &str) -> Result<TextureId> {
        held.textures
            .iter()
            .find(|(t, _)| t == name)
            .map(|(_, texture)| texture.id)
            .ok_or(Error::Invalid("an attachment names no texture"))
    }

    /// Encodes a render step (a render case is one) into `commands`, over
    /// the case's resources in `held`.
    pub(crate) fn encode_render(
        &mut self,
        step: &Value,
        library: &str,
        held: &Held,
        commands: &mut CommandBuffer,
    ) -> Result<()> {
        let case = step;
        let render = &step["render"];
        let empty = Map::new();

        // The pipeline's targets and the pass's attachments.
        let library = self.library(library)?;
        let constants = super::suite_constants::values(&case["constants"])?;
        let mesh = &render["mesh"];
        let tess = &render["tessellation"];
        let mut function = |name: &Value| -> Result<_> {
            self.device
                .new_function(library, name.as_str().unwrap_or_default(), Some(&constants))
        };
        // A mesh pipeline has object and mesh functions in place of a vertex one.
        let vertex = if mesh.is_object() {
            None
        } else {
            Some(function(&case["vertex"])?)
        };
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
            let mut request = attachment(texture, clear, [0.0; 4]);
            // The level, slice or 3D plane the attachment renders into.
            request.level = spec["level"].as_u64().unwrap_or(0);
            request.slice = spec["slice"].as_u64().unwrap_or(0);
            request.depth_plane = spec["depth_plane"].as_u64().unwrap_or(0);
            if let Some(resolve) = spec["resolve"].as_str() {
                let number = |key: &str| spec[key].as_u64().unwrap_or(0);
                request.resolve = Some((
                    Self::texture_named(held, resolve)?,
                    number("resolve_level"),
                    number("resolve_slice"),
                    number("resolve_depth_plane"),
                ));
                request.store_action = if spec["store"] == "resolve" {
                    StoreAction::MultisampleResolve
                } else {
                    StoreAction::StoreAndMultisampleResolve
                };
            }
            colors[index] = Some(request);
        }
        let mut depth_stencil_descriptor = DepthStencilDescriptor::default();
        let mut depth_filter = MultisampleDepthResolveFilter::Sample0;
        let mut stencil_filter = MultisampleStencilResolveFilter::Sample0;
        let mut depth_attachment = None;
        let depth = &render["depth"];
        if depth.is_object() {
            let texture = Self::texture_named(held, depth["texture"].as_str().unwrap_or_default())?;
            targets.depth_attachment_pixel_format =
                Some(self.device.texture_descriptor(texture)?.pixel_format);
            let mut request = attachment(texture, depth["clear"].as_f64(), 0.0);
            if let Some(resolve) = depth["resolve"].as_str() {
                request.resolve = Some((Self::texture_named(held, resolve)?, 0, 0, 0));
                request.store_action = StoreAction::StoreAndMultisampleResolve;
                depth_filter = match depth["resolve_filter"].as_str().unwrap_or("sample0") {
                    "min" => MultisampleDepthResolveFilter::Min,
                    "max" => MultisampleDepthResolveFilter::Max,
                    _ => MultisampleDepthResolveFilter::Sample0,
                };
            }
            depth_attachment = Some(request);
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
            let mut request = attachment(texture, clear, 0);
            if let Some(resolve) = stencil["resolve"].as_str() {
                request.resolve = Some((Self::texture_named(held, resolve)?, 0, 0, 0));
                request.store_action = StoreAction::StoreAndMultisampleResolve;
                if stencil["resolve_filter"] == "depth_resolved" {
                    stencil_filter = MultisampleStencilResolveFilter::DepthResolvedSample;
                }
            }
            stencil_attachment = Some(request);
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
        let samples = render["samples"].as_u64().unwrap_or(1);
        targets.raster_sample_count = samples;
        targets.alpha_to_coverage_enabled = render["alpha_to_coverage"] == true;
        targets.alpha_to_one_enabled = render["alpha_to_one"] == true;
        let vertex_descriptor = if render["vertex_descriptor"].is_object() {
            Some(vertex_descriptor(&render["vertex_descriptor"])?)
        } else {
            None
        };
        // The functions each stage's tables call.
        let (vertex_links, vertex_linked) = self.linked_functions(library, render, "vertex_")?;
        let (fragment_links, fragment_linked) =
            self.linked_functions(library, render, "fragment_")?;
        let targets_formats: [Option<stela_driver_gpu::metal::pixel_format::PixelFormat>;
            MAX_COLOR_ATTACHMENTS] =
            core::array::from_fn(|at| targets.color_attachments[at].pixel_format);
        let pipeline = match vertex {
            Some(vertex) => self.device.new_render_pipeline(&RenderPipelineRequest {
                vertex_function: vertex,
                fragment_function: fragment,
                vertex_descriptor,
                targets,
                input_primitive_topology: match render["input_primitive"].as_str() {
                    Some(class) => named(PrimitiveTopologyClass::ALL, class)
                        .ok_or(Error::Unsupported("a primitive class"))?,
                    None => PrimitiveTopologyClass::Unspecified,
                },
                max_vertex_amplification_count: render["amplification"]["count"]
                    .as_u64()
                    .unwrap_or(1),
                tessellation: if tess.is_object() {
                    tessellation(tess)?
                } else {
                    Default::default()
                },
                vertex_linked_functions: vertex_links,
                fragment_linked_functions: fragment_links,
                support_indirect_command_buffers: render["supports_icb"] == true,
                archives: PipelineArchives::default(),
            })?,
            None => {
                let library_functions = (
                    match mesh["object"].as_str() {
                        Some(name) => {
                            Some(self.device.new_function(library, name, Some(&constants))?)
                        }
                        None => None,
                    },
                    self.device.new_function(
                        library,
                        mesh["mesh"].as_str().unwrap_or_default(),
                        Some(&constants),
                    )?,
                );
                let threads = StageThreads {
                    max_total_threads: 0,
                    multiple_of_width: mesh["group_multiple_of_width"] == true,
                };
                self.device.new_mesh_render_pipeline(&MeshPipelineRequest {
                    object_function: library_functions.0,
                    mesh_function: library_functions.1,
                    fragment_function: fragment,
                    targets,
                    max_vertex_amplification_count: 1,
                    object_threads: threads,
                    mesh_threads: threads,
                    payload_memory_length: mesh["payload_length"].as_u64().unwrap_or(0),
                    object_linked_functions: LinkedFunctions::default(),
                    mesh_linked_functions: LinkedFunctions::default(),
                    fragment_linked_functions: LinkedFunctions::default(),
                    archives: PipelineArchives::default(),
                })?
            }
        };
        let state = self.device.render_pipeline_state(pipeline)?.clone();
        let tiles = self.tile_pipelines(render, library, &constants, &targets_formats)?;
        // An array target renders layered, the vertex function choosing the
        // slice, unless the attachment names one.
        let mut layers = 0;
        for request in colors
            .iter()
            .flatten()
            .zip(render["colors"].as_array().into_iter().flatten())
            .filter(|(_, spec)| spec["slice"].is_null())
            .map(|(c, _)| c.texture)
            .chain(depth_attachment.as_ref().map(|d| d.texture))
        {
            let descriptor = self.device.texture_descriptor(request)?;
            if matches!(
                descriptor.texture_type,
                TextureType::D2Array | TextureType::D2MultisampleArray
            ) {
                layers = descriptor.array_length;
            }
        }
        let size = |at: usize| render["target_size"][at].as_u64().unwrap_or(0);
        let pass = RenderPassRequest {
            color_attachments: colors,
            depth_attachment,
            depth_resolve_filter: depth_filter,
            stencil_attachment,
            stencil_resolve_filter: stencil_filter,
            visibility_result_buffer: match render["visibility"]["buffer"].as_str() {
                Some(name) => Some(
                    held.buffers[*held
                        .named
                        .get(name)
                        .ok_or(Error::Invalid("the visibility buffer is no buffer"))?]
                    .id,
                ),
                None => None,
            },
            visibility_result_type: if render["visibility"]["type"] == "accumulate" {
                VisibilityResultType::Accumulate
            } else {
                VisibilityResultType::Reset
            },
            render_target_array_length: layers,
            render_target_width: size(0),
            render_target_height: size(1),
            default_raster_sample_count: if render["target_size"].is_array() {
                samples
            } else {
                0
            },
            tile_memory: super::suite_tile::tile_memory(render),
            sample_positions: render["sample_positions"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|p| [0, 1].map(|at| p[at].as_f64().unwrap_or(0.5) as f32))
                .collect(),
            rasterization_rate_map: false,
            counter_samples: Vec::new(),
        };

        // The encoder's state, the bindings, the draws.
        self.device.begin_render(commands, &pass)?;
        commands.set_render_pipeline_state(&state)?;
        self.bind_render_tables(
            render,
            pipeline,
            [&vertex_linked, &fragment_linked],
            commands,
        )?;
        // Each view's offsets into the viewports and target layers.
        if render["amplification"].is_object() {
            let mappings: Vec<(u32, u32)> = render["amplification"]["mappings"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|m| {
                    (
                        m[0].as_u64().unwrap_or(0) as u32,
                        m[1].as_u64().unwrap_or(0) as u32,
                    )
                })
                .collect();
            commands.set_vertex_amplification(&mappings)?;
        }
        if !tiles.is_empty() {
            self.bind_tile_resources(case, held, commands)?;
            self.dispatch_tiles(&tiles, true, commands)?;
            commands.set_render_pipeline_state(&state)?;
        }
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
        if render["depth_clip"] == "clamp" {
            commands.set_depth_clip_mode(DepthClipMode::Clamp)?;
        }
        if render["fill"] == "lines" {
            commands.set_triangle_fill_mode(TriangleFillMode::Lines)?;
        }
        if let Some(list) = render["viewports"].as_array() {
            let viewports: Vec<Viewport> = list
                .iter()
                .take(16)
                .map(|v| {
                    let value = |at: usize| v[at].as_f64().unwrap_or(0.0);
                    Viewport {
                        origin_x: value(0),
                        origin_y: value(1),
                        width: value(2),
                        height: value(3),
                        znear: 0.0,
                        zfar: 1.0,
                    }
                })
                .collect();
            commands.set_viewports(&viewports)?;
        }
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
        self.bind_resources(case, held, commands, false)?;
        let primitive = named(
            PrimitiveType::ALL,
            render["primitive"].as_str().unwrap_or("triangle"),
        )
        .ok_or(Error::Unsupported("a primitive type"))?;
        let draws: Vec<&Map<String, Value>> = match render["draws"].as_array() {
            Some(draws) => draws.iter().filter_map(Value::as_object).collect(),
            None => vec![render.as_object().unwrap_or(&empty)],
        };
        let buffer_named = |name: &str| -> Result<_> {
            let at = *held
                .named
                .get(name)
                .ok_or(Error::Invalid("a draw names no buffer"))?;
            self.device.buffer(held.buffers[at].id)
        };
        if render["execute_icb"].is_object() {
            self.execute_icb(&render["execute_icb"], held, commands)?;
            return commands.end_encoding(&mut self.device.resolver());
        }
        if mesh.is_object() {
            let size = |key: &str| -> [u32; 3] {
                core::array::from_fn(|at| mesh[key][at].as_u64().unwrap_or(1) as u32)
            };
            if let Some(length) = mesh["object_threadgroup_memory"].as_u64() {
                commands.set_object_threadgroup_memory_length(length, 0)?;
            }
            let (object, mesh_threads) = (size("object_threads"), size("mesh_threads"));
            let indirect = mesh["indirect"].as_str().map(buffer_named).transpose()?;
            let mut resolver = self.device.resolver();
            if let Some(arguments) = indirect {
                commands.draw_mesh_threadgroups_indirect(
                    &mut resolver,
                    arguments,
                    0,
                    object,
                    mesh_threads,
                )?;
            } else if mesh["threads"].is_array() {
                commands.draw_mesh_threads(&mut resolver, size("threads"), object, mesh_threads)?;
            } else {
                commands.draw_mesh_threadgroups(
                    &mut resolver,
                    size("threadgroups"),
                    object,
                    mesh_threads,
                )?;
            }
            drop(resolver);
            return commands.end_encoding(&mut self.device.resolver());
        }
        if tess.is_object() {
            // Factors per patch, and per instance at a stride; patches by
            // index buffer and draws from an indirect buffer when named.
            let factors = buffer_named(tess["factors"].as_str().unwrap_or_default())?;
            commands.set_tessellation_factor_buffer(
                Some(factors),
                0,
                tess["instance_stride"].as_u64().unwrap_or(0),
            )?;
            if let Some(scale) = tess["factor_scale"].as_f64() {
                commands.set_tessellation_factor_scale(scale as f32)?;
            }
            let indices = |key: &str| -> Result<Option<_>> {
                tess[key]
                    .as_str()
                    .map(|name| buffer_named(name).map(|buffer| (buffer, 0)))
                    .transpose()
            };
            let source = PatchSource {
                control_points: tess["control_points"].as_u64().unwrap_or(4) as u32,
                patch_indices: indices("patch_index")?,
                control_point_indices: indices("control_point_index")?,
            };
            let indirect = tess["indirect"].as_str().map(buffer_named).transpose()?;
            let mut resolver = self.device.resolver();
            if let Some(arguments) = indirect {
                commands.draw_patches_indirect(&mut resolver, &source, arguments, 0)?;
            } else {
                let number = |key: &str, default: u64| tess[key].as_u64().unwrap_or(default) as u32;
                commands.draw_patches(
                    &mut resolver,
                    &source,
                    number("patch_start", 0),
                    number("patch_count", 1),
                    number("instance_count", 1),
                    0,
                )?;
            }
            drop(resolver);
            return commands.end_encoding(&mut self.device.resolver());
        }
        if render["indirect"].is_object() {
            let indirect = &render["indirect"];
            let arguments = buffer_named(indirect["buffer"].as_str().unwrap_or_default())?;
            let offset = indirect["offset"].as_u64().unwrap_or(0);
            if let Some(name) = render["index_buffer"].as_str() {
                let index_type = if render["index_type"] == "uint32" {
                    IndexType::UInt32
                } else {
                    IndexType::UInt16
                };
                let indices = buffer_named(name)?;
                commands.draw_indexed_primitives_indirect(
                    &mut self.device.resolver(),
                    primitive,
                    index_type,
                    indices,
                    render["index_offset"].as_u64().unwrap_or(0),
                    arguments,
                    offset,
                )?;
            } else {
                commands.draw_primitives_indirect(
                    &mut self.device.resolver(),
                    primitive,
                    arguments,
                    offset,
                )?;
            }
            return commands.end_encoding(&mut self.device.resolver());
        }
        for draw in draws {
            // Each draw counted into its own visibility result.
            if render["visibility"].is_object() {
                let mode = draw
                    .get("visibility_mode")
                    .and_then(Value::as_str)
                    .or(render["visibility"]["mode"].as_str())
                    .unwrap_or("counting");
                let mode = named(VisibilityResultMode::ALL, mode)
                    .ok_or(Error::Unsupported("a visibility result mode"))?;
                let offset = draw
                    .get("visibility_offset")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                commands.set_visibility_result_mode(mode, offset)?;
            }
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
                    render["base_vertex"].as_i64().unwrap_or(0) as _,
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
        self.dispatch_tiles(&tiles, false, commands)?;
        commands.end_encoding(&mut self.device.resolver())
    }
}
