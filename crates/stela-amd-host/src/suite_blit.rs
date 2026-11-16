//! The suite's blit cases, as the Mac's runner encodes them (`encode_blits`
//! in `runner.m`): buffers and textures, then each blit in order on one
//! encoder.
use super::suite::{BASIC_OUTPUT, Held, Runner};
use super::suite_resources::blit_option;
use serde_json::Value;
use stela_driver_gpu::metal::{
    capabilities::FormatCapabilities,
    command_buffer::{
        CommandBuffer,
        blit::{BufferLayout, Region},
    },
    error::{Error, Result},
};

const OPS: [&str; 15] = [
    "copy_icb",
    "optimize_icb",
    "reset_icb",
    "buffer_to_texture",
    "buffer_to_texture_plane",
    "copy_buffer",
    "copy_slices",
    "copy_texture",
    "copy_whole_texture",
    "fill_buffer",
    "generate_mipmaps",
    "optimize_cpu_access",
    "optimize_gpu_access",
    "texture_to_buffer",
    "texture_to_buffer_plane",
];

fn only(value: &Value, keys: &[&str]) -> bool {
    value
        .as_object()
        .is_some_and(|object| object.keys().all(|key| keys.contains(&key.as_str())))
}

/// Whether this runner reproduces the blit case exactly.
pub fn basic(case: &Value) -> bool {
    let all = |value: &Value, keys: &[&str]| {
        value.is_null()
            || value
                .as_array()
                .is_some_and(|items| items.iter().all(|item| only(item, keys)))
    };
    case["kind"] == "blit"
        && only(
            case,
            &[
                "blits", "buffers", "icbs", "kind", "library", "name", "outputs", "requires",
                "textures",
            ],
        )
        && case["blits"].as_array().is_some_and(|ops| {
            ops.iter()
                .all(|op| OPS.contains(&op["op"].as_str().unwrap_or_default()))
        })
        && all(
            &case["buffers"],
            &["file", "index", "length", "name", "storage"],
        )
        && all(&case["textures"], &super::suite_resources::TEXTURE)
        && case["textures"].as_array().into_iter().flatten().all(|t| {
            super::suite_resources::texture_type(t["type"].as_str().unwrap_or("2d")).is_some()
        })
        && all(&case["outputs"], &BASIC_OUTPUT)
}

fn triple(value: &Value, default: u64) -> [u64; 3] {
    core::array::from_fn(|at| {
        value[at]
            .as_u64()
            .unwrap_or(if at == 0 { default } else { 1 })
    })
}

impl Runner<'_> {
    /// Encodes a blit step (a blit case is one) into `commands`.
    pub(crate) fn encode_blits(
        &mut self,
        step: &Value,
        held: &Held,
        commands: &mut CommandBuffer,
    ) -> Result<()> {
        commands.begin_blit()?;
        for op in step["blits"].as_array().into_iter().flatten() {
            self.blit(op, held, commands)?;
        }
        commands.end_encoding(&mut self.device.resolver())
    }

    fn blit(&mut self, op: &Value, held: &Held, commands: &mut CommandBuffer) -> Result<()> {
        // The indirect command buffer blits, over a range of commands.
        let icb = |key: &str| {
            let name = op[key].as_str().unwrap_or_default();
            held.icbs
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, id)| *id)
                .ok_or(Error::Invalid("a blit names no indirect command buffer"))
        };
        let start = op["start"].as_u64().unwrap_or(0) as usize;
        let range = start..start + op["count"].as_u64().unwrap_or(1) as usize;
        match op["op"].as_str() {
            Some("reset_icb") => {
                return commands.reset_commands(&mut self.device.resolver(), icb("icb")?, range);
            }
            Some("optimize_icb") => {
                return commands.optimize_indirect_commands(
                    &mut self.device.resolver(),
                    icb("icb")?,
                    range,
                );
            }
            Some("copy_icb") => {
                return commands.copy_indirect_commands(
                    &mut self.device.resolver(),
                    icb("src")?,
                    range,
                    icb("dst")?,
                    op["dst_start"].as_u64().unwrap_or(0) as usize,
                );
            }
            _ => {}
        }
        let number = |key: &str| op[key].as_u64().unwrap_or(0);
        let buffer = |key: &str| -> Result<_> {
            let name = op[key].as_str().unwrap_or_default();
            let at = *held
                .named
                .get(name)
                .ok_or(Error::Invalid("a blit names no buffer"))?;
            Ok(held.buffers[at].id)
        };
        let texture = |key: &str| {
            let name = op[key].as_str().unwrap_or_default();
            held.textures
                .iter()
                .find(|(t, _)| t == name)
                .map(|(_, texture)| texture)
                .ok_or(Error::Invalid("a blit names no texture"))
        };
        match op["op"].as_str().unwrap_or_default() {
            "copy_buffer" => {
                let (src, dst) = (
                    self.device.buffer(buffer("src")?)?,
                    self.device.buffer(buffer("dst")?)?,
                );
                commands.copy_buffer(
                    src,
                    number("src_offset"),
                    dst,
                    number("dst_offset"),
                    number("size"),
                )
            }
            "fill_buffer" => {
                let dst = self.device.buffer(buffer("dst")?)?;
                let value = number("value") as u8;
                commands.fill_buffer(
                    &mut self.device.resolver(),
                    dst,
                    number("offset"),
                    number("size"),
                    value,
                )
            }
            kind @ ("buffer_to_texture" | "buffer_to_texture_plane") => {
                let dst = texture("dst")?;
                self.device.note_texture_write(commands, dst.id);
                let source = BufferLayout {
                    buffer: self.device.buffer(buffer("src")?)?,
                    offset: number("src_offset"),
                    bytes_per_row: number("bytes_per_row"),
                    bytes_per_image: number("bytes_per_image"),
                };
                let aspect = (kind == "buffer_to_texture_plane")
                    .then(|| op["aspect"].as_str())
                    .flatten();
                let option = if kind == "buffer_to_texture_plane" {
                    blit_option(dst.descriptor.pixel_format, aspect)
                } else {
                    Default::default()
                };
                let region = Region {
                    slice: number("slice"),
                    level: number("level"),
                    origin: core::array::from_fn(|at| op["origin"][at].as_u64().unwrap_or(0)),
                    size: triple(&op["size"], 1),
                };
                let target = self.blit_at(dst, region.level, region.slice)?;
                commands.copy_buffer_to_texture(
                    &mut self.device.resolver(),
                    source,
                    target,
                    region,
                    option,
                )
            }
            kind @ ("texture_to_buffer" | "texture_to_buffer_plane") => {
                let src = texture("src")?;
                let layout = BufferLayout {
                    buffer: self.device.buffer(buffer("dst")?)?,
                    offset: number("dst_offset"),
                    bytes_per_row: number("bytes_per_row"),
                    bytes_per_image: number("bytes_per_image"),
                };
                let option = if kind == "texture_to_buffer_plane" {
                    blit_option(src.descriptor.pixel_format, op["aspect"].as_str())
                } else {
                    Default::default()
                };
                let region = Region {
                    slice: number("slice"),
                    level: number("level"),
                    origin: core::array::from_fn(|at| op["origin"][at].as_u64().unwrap_or(0)),
                    size: triple(&op["size"], 1),
                };
                let source = self.blit_at(src, region.level, region.slice)?;
                commands.copy_texture_to_buffer(
                    &mut self.device.resolver(),
                    source,
                    region,
                    layout,
                    option,
                )
            }
            "copy_texture" => {
                let (src, dst) = (texture("src")?, texture("dst")?);
                self.device.note_texture_write(commands, dst.id);
                let region = Region {
                    slice: number("src_slice"),
                    level: number("src_level"),
                    origin: core::array::from_fn(|at| op["src_origin"][at].as_u64().unwrap_or(0)),
                    size: triple(&op["size"], 1),
                };
                let source = self.blit_at(src, region.level, region.slice)?;
                let target = self.blit_at(dst, number("dst_level"), number("dst_slice"))?;
                let origin = core::array::from_fn(|at| op["dst_origin"][at].as_u64().unwrap_or(0));
                commands.copy_texture(
                    source,
                    region,
                    target,
                    number("dst_slice"),
                    number("dst_level"),
                    origin,
                )
            }
            kind @ ("copy_whole_texture" | "copy_slices") => {
                let (src, dst) = (texture("src")?, texture("dst")?);
                self.device.note_texture_write(commands, dst.id);
                let whole = kind == "copy_whole_texture";
                let slices = if whole {
                    src.descriptor.slice_count()
                } else {
                    op["slices"].as_u64().unwrap_or(1)
                };
                let levels = if whole {
                    src.descriptor.mipmap_level_count
                } else {
                    op["levels"].as_u64().unwrap_or(1)
                };
                let source = self.blit_at(src, number("src_level"), number("src_slice"))?;
                let target = self.blit_at(dst, number("dst_level"), number("dst_slice"))?;
                commands.copy_texture_slices(
                    source,
                    number("src_slice"),
                    number("src_level"),
                    target,
                    number("dst_slice"),
                    number("dst_level"),
                    slices,
                    levels,
                )
            }
            "generate_mipmaps" => {
                let target = texture("texture")?;
                self.device.note_texture_write(commands, target.id);
                let filterable = self
                    .device
                    .capabilities()
                    .format(target.descriptor.pixel_format)
                    .contains(FormatCapabilities::FILTER);
                let texture = self.blit_at(target, 0, 0)?;
                commands.generate_mipmaps(texture, filterable)
            }
            // Hints, which leave the contents as they are.
            "optimize_cpu_access" | "optimize_gpu_access" => Ok(()),
            _ => Err(Error::Unsupported("a blit this runner does not encode")),
        }
    }
}
