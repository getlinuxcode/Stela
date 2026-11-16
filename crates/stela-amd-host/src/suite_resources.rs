//! Textures and samplers of the suite's cases, as the Mac's runner makes
//! them (`texture_descriptor`, `make_texture`, `regions_of`, `make_sampler`
//! in `runner.m`): textures are private, their files hold every slice's
//! levels with tight rows, loaded through a staging buffer and read back
//! the same way.
use super::suite::{Runner, Shared, named};
use serde_json::Value;
use stela_driver_gpu::metal::{
    abi::{
        BlitOption, CompareFunction, CpuCacheMode, HazardTrackingMode, ResourceOptions,
        SamplerAddressMode, SamplerBorderColor, SamplerMinMagFilter, SamplerMipFilter, SamplerReductionMode, StorageMode,
        SwizzleChannels, TextureSwizzle, TextureType, TextureUsage,
    },
    binding::ResourceId,
    command_buffer::{
        CommandBuffer,
        blit::{BlitTexture, BufferLayout, Region},
    },
    device::{SamplerId, TextureId},
    error::{Error, Result},
    pixel_format::PixelFormat,
    resource::{TextureDescriptor, TextureViewDescriptor},
    sampler::SamplerDescriptor,
};

/// The texture fields this runner reproduces (`storage`, `row_padding` and
/// `write_combined` change where the CPU reaches the texels, not what they
/// hold, and this runner loads every texture through a blit).
pub const TEXTURE: [&str; 20] = [
    "write_combined",
    "levels",
    "slices",
    "swizzle",
    "view_of",
    "array",
    "samples",
    "stencil_file",
    "depth",
    "file",
    "format",
    "height",
    "index",
    "mipmaps",
    "name",
    "row_padding",
    "storage",
    "type",
    "usage",
    "width",
];
pub const SAMPLER: [&str; 16] = [
    "address",
    "argument_buffers",
    "reduction",
    "bind_lod",
    "anisotropy",
    "border",
    "compare",
    "index",
    "lod_average",
    "lod_max",
    "lod_min",
    "mag",
    "min",
    "mip",
    "name",
    "normalized",
];

/// The texture types this runner makes.
pub fn texture_type(name: &str) -> Option<TextureType> {
    Some(match name {
        "1d" => TextureType::D1,
        "1d_array" => TextureType::D1Array,
        "2d" => TextureType::D2,
        "2d_array" => TextureType::D2Array,
        "3d" => TextureType::D3,
        "cube" => TextureType::Cube,
        "cube_array" => TextureType::CubeArray,
        "2d_ms" => TextureType::D2Multisample,
        "2d_ms_array" => TextureType::D2MultisampleArray,
        _ => return None,
    })
}

/// One level of one slice: where its texels sit in the case's file.
pub struct Subresource {
    pub slice: u64,
    pub level: u64,
    pub offset: u64,
    pub bytes_per_row: u64,
    pub bytes_per_image: u64,
    pub size: [u64; 3],
}

/// A texture the case made.
pub struct Texture {
    pub id: TextureId,
    pub descriptor: TextureDescriptor,
}

/// Whether `format` holds depth and stencil, whose aspects copy apart.
fn combined(format: PixelFormat) -> bool {
    matches!(
        format,
        PixelFormat::Depth32FloatStencil8 | PixelFormat::Depth24UnormStencil8
    )
}

/// The blit option that copies `aspect` of `format`, as the runner's
/// `blit_option`.
pub fn blit_option(format: PixelFormat, aspect: Option<&str>) -> BlitOption {
    match (combined(format), aspect) {
        (true, Some("stencil")) => BlitOption::STENCIL_FROM_DEPTH_STENCIL,
        (true, _) => BlitOption::DEPTH_FROM_DEPTH_STENCIL,
        (false, _) => BlitOption::EMPTY,
    }
}

impl Texture {
    /// Every slice's levels in the runner's order, and their total bytes, of
    /// `aspect` of a depth and stencil format (depth without one).
    pub fn regions(&self, aspect: Option<&str>) -> (Vec<Subresource>, u64) {
        let d = &self.descriptor;
        let mut block = d.pixel_format.describe().block;
        if combined(d.pixel_format) {
            block.bytes = if aspect == Some("stencil") { 1 } else { 4 };
        }
        let cube = matches!(d.texture_type, TextureType::Cube | TextureType::CubeArray);
        let slices = d.array_length * if cube { 6 } else { 1 };
        let extent = |size: u64, level: u64| (size >> level).max(1);
        let mut out = Vec::new();
        let mut offset = 0;
        for slice in 0..slices {
            for level in 0..d.mipmap_level_count {
                let size = [
                    extent(d.width, level),
                    extent(d.height, level),
                    extent(d.depth, level),
                ];
                let bytes_per_row =
                    size[0].div_ceil(u64::from(block.width)) * u64::from(block.bytes);
                let bytes_per_image = bytes_per_row * size[1].div_ceil(u64::from(block.height));
                out.push(Subresource {
                    slice,
                    level,
                    offset,
                    bytes_per_row,
                    bytes_per_image,
                    size,
                });
                offset += bytes_per_image * size[2];
            }
        }
        (out, offset)
    }
}

impl Runner<'_> {
    /// Makes the case's textures, and records into `commands` a blit that
    /// loads those with a file.
    pub(crate) fn make_textures(
        &mut self,
        specs: &Value,
        commands: &mut CommandBuffer,
        buffers: &mut Vec<Shared>,
    ) -> Result<Vec<(String, Texture)>> {
        let mut textures = Vec::new();
        let mut uploads = Vec::new();
        for spec in specs.as_array().into_iter().flatten() {
            if spec["view_of"].is_string() {
                let view = self.make_view(spec, &textures)?;
                textures.push((spec["name"].as_str().unwrap_or_default().to_owned(), view));
                continue;
            }
            let format = spec["format"].as_str().unwrap_or_default();
            let pixel_format = named(PixelFormat::ALL, format).ok_or(Error::Unsupported(
                "a pixel format this runner does not name",
            ))?;
            let texture_type = texture_type(spec["type"].as_str().unwrap_or("2d")).ok_or(
                Error::Unsupported("a texture type this runner does not make"),
            )?;
            let mut usage = TextureUsage::EMPTY;
            for item in spec["usage"].as_array().into_iter().flatten() {
                usage |= match item.as_str() {
                    Some("read") => TextureUsage::SHADER_READ,
                    Some("write") => TextureUsage::SHADER_WRITE,
                    Some("render") => TextureUsage::RENDER_TARGET,
                    Some("atomic") => TextureUsage::SHADER_ATOMIC,
                    Some("view") => TextureUsage::PIXEL_FORMAT_VIEW,
                    _ => return Err(Error::Unsupported("a texture usage")),
                };
            }
            if spec["usage"].is_null() {
                usage = TextureUsage::SHADER_READ;
            }
            let number = |key: &str| spec[key].as_u64().unwrap_or(1);
            let descriptor = TextureDescriptor {
                texture_type,
                pixel_format,
                width: number("width"),
                height: number("height"),
                depth: number("depth"),
                mipmap_level_count: number("mipmaps"),
                sample_count: number("samples"),
                array_length: number("array"),
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
            for (key, aspect) in [
                ("file", spec["aspect"].as_str()),
                ("stencil_file", Some("stencil")),
            ] {
                let Some(file) = spec[key].as_str() else {
                    continue;
                };
                let data = self.input(&serde_json::json!({ "file": file }))?;
                let (_, total) = texture.regions(aspect);
                if (data.len() as u64) < total {
                    return Err(Error::Invalid("a texture input is short"));
                }
                let staging = self.shared(&data[..total as usize], total)?;
                uploads.push((textures.len(), buffers.len(), aspect.map(str::to_owned)));
                buffers.push(staging);
            }
            textures.push((
                spec["name"].as_str().unwrap_or_default().to_owned(),
                texture,
            ));
        }
        if !uploads.is_empty() {
            commands.begin_blit()?;
            for (at, staging, aspect) in uploads {
                let texture = &textures[at].1;
                let aspect = aspect.as_deref();
                let (regions, _) = texture.regions(aspect);
                for region in regions {
                    let source = BufferLayout {
                        buffer: self.device.buffer(buffers[staging].id)?,
                        offset: region.offset,
                        bytes_per_row: region.bytes_per_row,
                        bytes_per_image: region.bytes_per_image,
                    };
                    let target = self.blit_texture(texture, &region)?;
                    commands.copy_buffer_to_texture(
                        &mut self.device.resolver(),
                        source,
                        target,
                        whole(&region),
                        blit_option(texture.descriptor.pixel_format, aspect),
                    )?;
                }
            }
            commands.end_encoding(&mut self.device.resolver())?;
        }
        Ok(textures)
    }

    fn blit_texture<'t>(
        &mut self,
        texture: &'t Texture,
        region: &Subresource,
    ) -> Result<BlitTexture<'t>> {
        self.blit_at(texture, region.level, region.slice)
    }

    /// `texture` for a blit of its `level` of `slice`.
    pub(crate) fn blit_at<'t>(
        &mut self,
        texture: &'t Texture,
        level: u64,
        slice: u64,
    ) -> Result<BlitTexture<'t>> {
        Ok(BlitTexture {
            image: self.device.texture_image(texture.id)?,
            descriptor: &texture.descriptor,
            storage: self.device.texel_storage(texture.id, level, slice)?,
            blocks: self.device.texture_blocks(texture.id)?,
        })
    }

    /// Records into `commands` the copy of `texture` into a new shared
    /// buffer, which it returns.
    pub(crate) fn read_back(
        &mut self,
        texture: &Texture,
        aspect: Option<&str>,
        commands: &mut CommandBuffer,
    ) -> Result<Shared> {
        let (regions, total) = texture.regions(aspect);
        let readback = self.shared(&[], total)?;
        for region in regions {
            let source = self.blit_texture(texture, &region)?;
            let layout = BufferLayout {
                buffer: self.device.buffer(readback.id)?,
                offset: region.offset,
                bytes_per_row: region.bytes_per_row,
                bytes_per_image: region.bytes_per_image,
            };
            commands.copy_texture_to_buffer(
                &mut self.device.resolver(),
                source,
                whole(&region),
                layout,
                blit_option(texture.descriptor.pixel_format, aspect),
            )?;
        }
        Ok(readback)
    }

    /// Makes a sampler as the case describes it.
    pub(crate) fn make_sampler(&mut self, spec: &Value) -> Result<(SamplerId, ResourceId)> {
        let text =
            |key: &str, default: &'static str| spec[key].as_str().unwrap_or(default).to_owned();
        let filter = |name: String| {
            named(SamplerMinMagFilter::ALL, &name).ok_or(Error::Unsupported("a sampler filter"))
        };
        let mip = match text("mip", "none").as_str() {
            "none" => SamplerMipFilter::NotMipmapped,
            name => named(SamplerMipFilter::ALL, name).ok_or(Error::Unsupported("a mip filter"))?,
        };
        let address: Vec<String> = match spec["address"].as_array() {
            Some(modes) => modes
                .iter()
                .map(|m| m.as_str().unwrap_or_default().to_owned())
                .collect(),
            None => vec!["clamp_to_edge".to_owned()],
        };
        let mode = |at: usize| {
            let name = &address[at.min(address.len() - 1)];
            match name.as_str() {
                "clamp_to_border" => Ok(SamplerAddressMode::ClampToBorderColor),
                name => named(SamplerAddressMode::ALL, name)
                    .ok_or(Error::Unsupported("a sampler address mode")),
            }
        };
        let mut descriptor = SamplerDescriptor {
            min_filter: filter(text("min", "nearest"))?,
            mag_filter: filter(text("mag", "nearest"))?,
            mip_filter: mip,
            max_anisotropy: spec["anisotropy"].as_u64().unwrap_or(1),
            address_modes: [mode(0)?, mode(1)?, mode(2)?],
            border_color: named(
                SamplerBorderColor::ALL,
                &text("border", "transparent_black"),
            )
            .ok_or(Error::Unsupported("a border color"))?,
            reduction_mode: named(
                SamplerReductionMode::ALL,
                &text("reduction", "weighted_average"),
            )
            .ok_or(Error::Unsupported("a reduction mode"))?,
            normalized_coordinates: spec["normalized"].as_bool().unwrap_or(true),
            lod_min_clamp: spec["lod_min"].as_f64().unwrap_or(0.0) as f32,
            lod_max_clamp: spec["lod_max"].as_f64().map_or(f32::MAX, |lod| lod as f32),
            ..SamplerDescriptor::default()
        };
        if let Some(compare) = spec["compare"].as_str() {
            descriptor.compare_function = named(CompareFunction::ALL, compare)
                .ok_or(Error::Unsupported("a compare function"))?;
        }
        let id = self.device.new_sampler(&descriptor)?;
        Ok((id, self.device.sampler_resource_id(id)?))
    }
}

/// The whole of one subresource.
fn whole(region: &Subresource) -> Region {
    Region {
        slice: region.slice,
        level: region.level,
        origin: [0; 3],
        size: region.size,
    }
}

impl Runner<'_> {
    /// A view of an earlier texture of the case: another format, type,
    /// mipmap and slice range or component swizzle over its memory.
    fn make_view(&mut self, spec: &Value, textures: &[(String, Texture)]) -> Result<Texture> {
        let source = spec["view_of"].as_str().unwrap_or_default();
        let parent = textures
            .iter()
            .find(|(name, _)| name == source)
            .map(|(_, texture)| texture)
            .ok_or(Error::Invalid("a view names no texture"))?;
        let p = &parent.descriptor;
        let range = |key: &str, whole: u64| -> core::ops::Range<u64> {
            let at = |i: usize, default: u64| spec[key][i].as_u64().unwrap_or(default);
            let start = at(0, 0);
            start..start + at(1, whole)
        };
        let cube = matches!(p.texture_type, TextureType::Cube);
        let levels = range("levels", p.mipmap_level_count);
        let slices = range("slices", if cube { 6 } else { p.array_length });
        let mut swizzle = SwizzleChannels::IDENTITY;
        let letters = spec["swizzle"].as_str().unwrap_or("rgba");
        for (at, letter) in letters.chars().take(4).enumerate() {
            swizzle.0[at] = match letter {
                'r' => TextureSwizzle::Red,
                'g' => TextureSwizzle::Green,
                'b' => TextureSwizzle::Blue,
                'a' => TextureSwizzle::Alpha,
                '0' => TextureSwizzle::Zero,
                '1' => TextureSwizzle::One,
                _ => return Err(Error::Unsupported("a swizzle component")),
            };
        }
        let pixel_format = match spec["format"].as_str() {
            Some(format) => named(PixelFormat::ALL, format)
                .ok_or(Error::Unsupported("a pixel format this runner does not name"))?,
            None => p.pixel_format,
        };
        let texture_type = match spec["type"].as_str() {
            Some(name) => texture_type(name)
                .ok_or(Error::Unsupported("a texture type this runner does not make"))?,
            None => p.texture_type,
        };
        let id = self.device.new_texture_view(
            parent.id,
            &TextureViewDescriptor {
                pixel_format,
                texture_type,
                levels: levels.clone(),
                slices: slices.clone(),
                swizzle,
                min_lod: 0.0,
            },
        )?;
        // What the view spans, which reading it back follows.
        let extent = |size: u64| (size >> levels.start).max(1);
        let view_cube = matches!(texture_type, TextureType::Cube | TextureType::CubeArray);
        let descriptor = TextureDescriptor {
            texture_type,
            pixel_format,
            width: extent(p.width),
            height: extent(p.height),
            depth: extent(p.depth),
            mipmap_level_count: levels.end - levels.start,
            array_length: (slices.end - slices.start) / if view_cube { 6 } else { 1 },
            swizzle,
            ..*p
        };
        Ok(Texture { id, descriptor })
    }
}
