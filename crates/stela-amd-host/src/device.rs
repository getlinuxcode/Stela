//! `--device`: opens Stela's real Metal device on the Radeon, over RADV built
//! from Stela's Mesa with the AIR compiler linked into the same library
//! (`libstela_radv.so`, Meson option `radv-host`), and runs Metal work on it.
//!
//! The Metal device reaches the guest's shared memory as dma-bufs it imports
//! ([`MemoryExporter`]). Here the "guest" is this process: each export is a
//! memfd turned into a dma-buf by `/dev/udmabuf`, which needs the `kvm`
//! group, and the host reads and writes the buffers through the memfd's
//! mapping.
use core::{ffi::c_void, num::NonZeroU64, ptr::NonNull};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use stela_contracts::gate::MemoryRange;
use stela_driver_gpu::metal::{
    abi::{
        BlitOption, CpuCacheMode, DispatchType, HazardTrackingMode, LoadAction,
        MultisampleDepthResolveFilter, MultisampleStencilResolveFilter, PrimitiveTopologyClass,
        PrimitiveType, ResourceOptions, StorageMode, StoreAction, StoreActionOptions,
        SwizzleChannels, TextureType, TextureUsage, VisibilityResultType,
    },
    amd::open::{Gpu, reopen},
    capabilities::Size,
    command_buffer::blit::{BlitTexture, BufferLayout, Region},
    compiler::{AirCompiler, Service},
    device::{
        BufferId, Device, LibraryId,
        pipelines::{
            ComputePipelineRequest, LinkedFunctions, PipelineArchives, RenderPipelineRequest,
        },
        queue::{AttachmentRequest, RenderPassRequest},
    },
    error::{Error, Result},
    native::{ExportedMemory, Loader, MemoryExporter, NativeBackend, vk},
    pipeline::{ColorAttachmentDescriptor, MAX_COLOR_ATTACHMENTS, RenderTargets},
    pixel_format::PixelFormat,
    resource::TextureDescriptor,
};

unsafe extern "C" {
    /// RADV's Vulkan entry point in `libstela_radv.so`.
    fn vk_icdGetInstanceProcAddr(
        instance: vk::Instance,
        name: *const core::ffi::c_char,
    ) -> vk::PFN_vkVoidFunction;
    /// The NIR options the AIR compiler builds RADV's shaders with.
    fn stela_radv_nir_options(physical: vk::PhysicalDevice) -> *const c_void;
}

pub(crate) type MetalDevice = Device<NativeBackend, AirCompiler>;

/// What `--device` and `--suite` run.
pub enum Work {
    /// The first work: a Metal library holding the shaders of
    /// `metal/first.metal`, compiled on a Mac.
    First(Vec<u8>),
    /// The Metal suite in a `metal-suite` directory, and the texts that
    /// select its cases.
    Suite(std::path::PathBuf, Vec<String>),
}

/// Opens the Metal device on `gpu` and runs `work` on it.
pub fn run(gpu: &Gpu, work: Work) -> core::result::Result<(), String> {
    let exporter = Exporter::default();
    let loader = Loader {
        get_instance_proc_addr: entry,
        log: super::log,
        submit_thread: None,
        refusals: None,
        relax: super::relax,
    };
    let compiler = |physical| {
        // SAFETY: RADV's own physical device, whose options it keeps.
        let options = unsafe { stela_radv_nir_options(physical) };
        Service::linked(NonNull::new(options.cast_mut()).expect("RADV has NIR options"))
    };
    // SAFETY: the loader and the compiler are libstela_radv's, which this
    // binary links, so they outlive the device.
    let mut device = unsafe { reopen(loader, Box::new(exporter.clone()), compiler, gpu) }
        .map_err(|error| format!("opening the Metal device: {error:?}"))?;
    println!("  Metal device: open on RADV, with the AIR compiler for GFX12");
    match work {
        Work::First(library) => {
            let library = device
                .new_library(library)
                .map_err(|error| format!("loading the Metal library: {error:?}"))?;
            add(&mut device, &exporter, library)
                .map_err(|error| format!("buffer sum: {error:?}"))?;
            triangle(&mut device, &exporter, library)
                .map_err(|error| format!("triangle: {error:?}"))
        }
        Work::Suite(dir, only) => super::suite::run(&mut device, &exporter, &dir, &only),
    }
}

/// The ICD's entry point as a loader's: `vk_icdGetInstanceProcAddr` answers
/// the global commands for a null instance, as `vkGetInstanceProcAddr` does.
unsafe extern "system" fn entry(
    instance: vk::Instance,
    name: *const core::ffi::c_char,
) -> vk::PFN_vkVoidFunction {
    // SAFETY: the caller's name is a valid string.
    unsafe { vk_icdGetInstanceProcAddr(instance, name) }
}

/// The sum of two buffers of floats, checked on the CPU.
fn add(device: &mut MetalDevice, exporter: &Exporter, library: LibraryId) -> Result<()> {
    const COUNT: usize = 1 << 16;
    let bytes = (COUNT * 4) as u64;
    let function = device.new_function(library, "add", Some(&[]))?;
    let pipeline = device.new_compute_pipeline(&ComputePipelineRequest {
        compute_function: function,
        threadgroup_size_is_multiple_of_thread_execution_width: false,
        max_total_threads_per_threadgroup: 0,
        stage_input: None,
        linked_functions: LinkedFunctions::default(),
        support_indirect_command_buffers: false,
        archives: PipelineArchives::default(),
    })?;
    let state = device.compute_pipeline_state(pipeline)?.clone();
    let [a, b, sum] = [1, 2, 3].map(|index| shared_buffer(device, exporter, index, bytes));
    let (a, b, sum) = (a?, b?, sum?);
    let floats = |(_, pointer): (BufferId, NonNull<u8>)| pointer.cast::<f32>().as_ptr();
    for index in 0..COUNT {
        // SAFETY: each mapping holds COUNT floats, which nothing else uses
        // until the commit.
        unsafe {
            *floats(a).add(index) = index as f32;
            *floats(b).add(index) = 0.5 * index as f32 + 1.0;
            *floats(sum).add(index) = -1.0;
        }
    }

    let mut commands = device.new_command_buffer();
    commands.begin_compute(DispatchType::Serial)?;
    commands.set_compute_pipeline_state(&state)?;
    for (index, (buffer, _)) in [a, b, sum].into_iter().enumerate() {
        commands.set_compute_buffer(Some(device.buffer(buffer)?), 0, index as u32)?;
    }
    let threads = Size {
        width: 64,
        height: 1,
        depth: 1,
    };
    commands.dispatch_threadgroups(&mut device.resolver(), [(COUNT / 64) as u32, 1, 1], threads)?;
    commands.end_encoding(&mut device.resolver())?;
    let started = Instant::now();
    let serial = device.commit(commands)?;
    wait(device, serial)?;
    let elapsed = started.elapsed();

    let mut wrong = 0;
    for index in 0..COUNT {
        let expected = 1.5 * index as f32 + 1.0;
        // SAFETY: the GPU finished writing the mapping.
        let found = unsafe { *floats(sum).add(index) };
        if found != expected {
            if wrong < 4 {
                println!("  sum[{index}] = {found}, expected {expected}");
            }
            wrong += 1;
        }
    }
    if wrong != 0 {
        println!("  buffer sum: {wrong} of {COUNT} wrong");
        return Err(Error::Invalid("the GPU's sum differs"));
    }
    println!("  buffer sum: {COUNT} floats added on the GPU and checked ({elapsed:.2?})");
    Ok(())
}

/// A triangle drawn into a 256 x 256 RGBA8 texture, read back through a
/// shared buffer and checked: black outside, each corner its vertex's color,
/// and the area a 0.75-extent triangle covers. Written to
/// `target/first-triangle.ppm` for a look.
const SIDE: usize = 256;

fn triangle(device: &mut MetalDevice, exporter: &Exporter, library: LibraryId) -> Result<()> {
    let vertex = device.new_function(library, "triangle_vertex", Some(&[]))?;
    let fragment = device.new_function(library, "triangle_fragment", Some(&[]))?;
    let mut targets = RenderTargets::default();
    targets.color_attachments[0] = ColorAttachmentDescriptor {
        pixel_format: Some(PixelFormat::Rgba8Unorm),
        ..ColorAttachmentDescriptor::default()
    };
    let pipeline = device.new_render_pipeline(&RenderPipelineRequest {
        vertex_function: vertex,
        fragment_function: Some(fragment),
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
    let state = device.render_pipeline_state(pipeline)?.clone();
    let (image, elapsed) = draw(device, exporter, &state, PrimitiveType::Triangle, 4)?;

    let pixel = |x: usize, y: usize| {
        let at = (y * SIDE + x) * 4;
        [image[at], image[at + 1], image[at + 2], image[at + 3]]
    };
    let mut ppm = format!("P6\n{SIDE} {SIDE}\n255\n").into_bytes();
    let mut covered = 0;
    for y in 0..SIDE {
        for x in 0..SIDE {
            let [r, g, b, _] = pixel(x, y);
            ppm.extend([r, g, b]);
            covered += usize::from(r as u32 + g as u32 + b as u32 > 0);
        }
    }
    let _ = std::fs::write("target/first-triangle.ppm", &ppm);
    // Clip space y is up and the framebuffer's y down: the red vertex is at
    // the top. Each probe sits well inside its corner.
    let probes = [
        ("outside", (8, 8), [0, 0, 0, 255], None),
        ("top, red", (128, 48), [0; 4], Some(0)),
        ("bottom left, green", (48, 216), [0; 4], Some(1)),
        ("bottom right, blue", (208, 216), [0; 4], Some(2)),
    ];
    let mut failed = false;
    for (name, (x, y), exact, dominant) in probes {
        let found = pixel(x, y);
        let good = match dominant {
            None => found == exact,
            Some(channel) => {
                (0..3).all(|other| other == channel || found[channel] > 2 * found[other])
                    && found[channel] > 128
            }
        };
        if !good {
            println!("  pixel {name} ({x}, {y}) = {found:?}");
            failed = true;
        }
    }
    // The triangle covers 1.5 x 1.5 / 2 of clip space's 2 x 2.
    let expected = SIDE * SIDE * 9 / 32;
    if covered.abs_diff(expected) > expected / 50 {
        println!("  {covered} pixels covered, expected about {expected}");
        failed = true;
    }
    if failed {
        return Err(Error::Invalid("the GPU's triangle differs"));
    }
    println!(
        "  triangle: drawn into a {SIDE}x{SIDE} texture and checked, {covered} pixels covered \
         ({elapsed:.2?}); image in target/first-triangle.ppm"
    );

    // The same pipeline, whose topology class is unspecified, draws a line
    // strip along two edges: on RADV, without unrestricted dynamic topology,
    // through the variant created for lines.
    let (image, elapsed) = draw(device, exporter, &state, PrimitiveType::LineStrip, 5)?;
    let lit = |x: usize, y: usize| image[(y * SIDE + x) * 4..][..3].iter().any(|&c| c > 0);
    let covered = (0..SIDE * SIDE)
        .filter(|at| lit(at % SIDE, at / SIDE))
        .count();
    // Two edges, of about 215 and 192 pixels.
    if !(350..=460).contains(&covered) || lit(128, 160) {
        println!("  line strip: {covered} pixels lit, expected about 405 and none inside");
        return Err(Error::Invalid("the GPU's line strip differs"));
    }
    println!(
        "  line strip: the same unspecified-class pipeline drew {covered} pixels ({elapsed:.2?})"
    );
    Ok(())
}

/// Draws `primitive` from the three vertices of `state` into a cleared
/// SIDE x SIDE RGBA8 texture, and reads it back through shared buffer
/// `index`: its pixels, and how long the GPU took.
fn draw<'a>(
    device: &mut MetalDevice,
    exporter: &'a Exporter,
    state: &stela_driver_gpu::metal::command_buffer::RenderPipelineState,
    primitive: PrimitiveType,
    index: u64,
) -> Result<(&'a [u8], Duration)> {
    let texture = device.new_texture(&TextureDescriptor {
        texture_type: TextureType::D2,
        pixel_format: PixelFormat::Rgba8Unorm,
        width: SIDE as u64,
        height: SIDE as u64,
        depth: 1,
        mipmap_level_count: 1,
        sample_count: 1,
        array_length: 1,
        options: ResourceOptions {
            cpu_cache_mode: CpuCacheMode::DefaultCache,
            storage_mode: StorageMode::Private,
            hazard_tracking_mode: HazardTrackingMode::Default,
        },
        usage: TextureUsage::RENDER_TARGET,
        swizzle: SwizzleChannels::IDENTITY,
    })?;
    let (readback, pixels) = shared_buffer(device, exporter, index, (SIDE * SIDE * 4) as u64)?;

    let mut colors = [None; MAX_COLOR_ATTACHMENTS];
    colors[0] = Some(AttachmentRequest {
        texture,
        level: 0,
        slice: 0,
        depth_plane: 0,
        resolve: None,
        load_action: LoadAction::Clear,
        store_action: StoreAction::Store,
        store_action_options: StoreActionOptions::EMPTY,
        clear: [0.0, 0.0, 0.0, 1.0],
        surface: None,
    });
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
    let mut commands = device.new_command_buffer();
    device.begin_render(&mut commands, &pass)?;
    commands.set_render_pipeline_state(state)?;
    commands.draw_primitives(&mut device.resolver(), primitive, 0, 3, 1, 0)?;
    commands.end_encoding(&mut device.resolver())?;
    commands.begin_blit()?;
    let descriptor = *device.texture_descriptor(texture)?;
    let source = BlitTexture {
        image: device.texture_image(texture)?,
        descriptor: &descriptor,
        storage: Default::default(),
        blocks: device.texture_blocks(texture)?,
    };
    let region = Region {
        slice: 0,
        level: 0,
        origin: [0; 3],
        size: [SIDE as u64, SIDE as u64, 1],
    };
    let layout = BufferLayout {
        buffer: device.buffer(readback)?,
        offset: 0,
        bytes_per_row: (SIDE * 4) as u64,
        bytes_per_image: 0,
    };
    commands.copy_texture_to_buffer(
        &mut device.resolver(),
        source,
        region,
        layout,
        BlitOption::EMPTY,
    )?;
    commands.end_encoding(&mut device.resolver())?;
    let started = Instant::now();
    let serial = device.commit(commands)?;
    wait(device, serial)?;
    let elapsed = started.elapsed();
    // SAFETY: the GPU finished writing the mapping, of SIDE x SIDE pixels,
    // which the exporter keeps for the process.
    let image = unsafe { core::slice::from_raw_parts(pixels.as_ptr(), SIDE * SIDE * 4) };
    Ok((image, elapsed))
}

/// Waits until the device completes `serial`.
pub(crate) fn wait(device: &mut MetalDevice, serial: u64) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while device.poll()? < serial {
        if Instant::now() > deadline {
            return Err(Error::Invalid("the GPU did not complete within 10 s"));
        }
        std::thread::sleep(Duration::from_micros(200));
    }
    Ok(())
}

/// A shared buffer of `bytes` over memory this process exports, and its
/// mapping.
pub(crate) fn shared_buffer(
    device: &mut MetalDevice,
    exporter: &Exporter,
    index: u64,
    bytes: u64,
) -> Result<(BufferId, NonNull<u8>)> {
    // Each buffer is its own fake guest range, by which the export is found.
    let address = index << 32;
    let token = NonZeroU64::new(index).expect("indices start at one");
    let memory = device.import_memory(&[MemoryRange { address, bytes }], token)?;
    let options = ResourceOptions {
        cpu_cache_mode: CpuCacheMode::DefaultCache,
        storage_mode: StorageMode::Shared,
        hazard_tracking_mode: HazardTrackingMode::Default,
    };
    let buffer = device.new_imported_buffer(memory, bytes, options)?;
    let pointer = exporter
        .mapping(address)
        .ok_or(Error::Invalid("the import made no export"))?;
    Ok((buffer, pointer))
}

/// Exports memfd pages as dma-bufs through `/dev/udmabuf`.
#[derive(Clone, Default)]
pub(crate) struct Exporter {
    /// Each export's mapping, by its first range's address.
    mappings: Arc<Mutex<BTreeMap<u64, usize>>>,
}

impl Exporter {
    fn mapping(&self, address: u64) -> Option<NonNull<u8>> {
        let mappings = self.mappings.lock().ok()?;
        NonNull::new(*mappings.get(&address)? as *mut u8)
    }
}

/// `struct udmabuf_create`.
#[repr(C)]
struct UdmabufCreate {
    memfd: u32,
    flags: u32,
    offset: u64,
    size: u64,
}

/// `UDMABUF_CREATE`: `_IOW('u', 0x42, struct udmabuf_create)`.
const UDMABUF_CREATE: libc::c_ulong = 0x4018_7542;
const UDMABUF_FLAGS_CLOEXEC: u32 = 1;

impl MemoryExporter for Exporter {
    fn export(&self, ranges: &[MemoryRange]) -> Result<ExportedMemory> {
        let first = ranges.first().ok_or(Error::Invalid("no range to export"))?;
        let requested: u64 = ranges.iter().map(|range| range.bytes).sum();
        let bytes = requested.next_multiple_of(4096);
        // SAFETY: plain system calls on descriptors this function owns.
        unsafe {
            let memfd = libc::memfd_create(
                c"stela-guest".as_ptr(),
                libc::MFD_ALLOW_SEALING | libc::MFD_CLOEXEC,
            );
            if memfd < 0 {
                return Err(Error::OutOfMemory);
            }
            let made = libc::ftruncate(memfd, bytes as libc::off_t) == 0
                && libc::fcntl(memfd, libc::F_ADD_SEALS, libc::F_SEAL_SHRINK) == 0;
            let device = libc::open(c"/dev/udmabuf".as_ptr(), libc::O_RDWR | libc::O_CLOEXEC);
            if !made || device < 0 {
                if device < 0 {
                    eprintln!("  /dev/udmabuf: cannot open it (run under the kvm group: sg kvm)");
                }
                libc::close(memfd);
                return Err(Error::Unsupported("udmabuf"));
            }
            let create = UdmabufCreate {
                memfd: memfd as u32,
                flags: UDMABUF_FLAGS_CLOEXEC,
                offset: 0,
                size: bytes,
            };
            let fd = libc::ioctl(device, UDMABUF_CREATE, &create);
            libc::close(device);
            let pointer = libc::mmap(
                core::ptr::null_mut(),
                bytes as usize,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                memfd,
                0,
            );
            libc::close(memfd);
            if fd < 0 || pointer == libc::MAP_FAILED {
                if fd >= 0 {
                    libc::close(fd);
                }
                return Err(Error::OutOfMemory);
            }
            let pointer = NonNull::new(pointer.cast::<u8>()).ok_or(Error::OutOfMemory)?;
            if let Ok(mut mappings) = self.mappings.lock() {
                mappings.insert(first.address, pointer.as_ptr() as usize);
            }
            Ok(ExportedMemory { fd, pointer, bytes })
        }
    }

    fn close(&self, exported: ExportedMemory) {
        // SAFETY: the export's own descriptor, which the driver did not take.
        unsafe { libc::close(exported.fd) };
        self.unmap(exported);
    }

    fn unmap(&self, exported: ExportedMemory) {
        if let Ok(mut mappings) = self.mappings.lock() {
            mappings.retain(|_, pointer| *pointer != exported.pointer.as_ptr() as usize);
        }
        // SAFETY: the export's own mapping, which nothing reads any more.
        unsafe { libc::munmap(exported.pointer.as_ptr().cast(), exported.bytes as usize) };
    }
}
