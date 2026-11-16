//! The heaps of the suite's cases, as the Mac's runner makes them
//! (`make_heap`, `allocate_buffer` in `runner.m`): automatic or placement,
//! shared or private, tracked or untracked. A shared heap is memory the
//! CPU reaches, imported as a guest's is; its buffers are mapped where the
//! heap places them.
use super::suite::{Held, Runner, Shared};
use core::ptr::NonNull;
use serde_json::Value;
use stela_driver_gpu::metal::{
    abi::{
        CpuCacheMode, HazardTrackingMode, HeapType, ResourceOptions, SparsePageSize, StorageMode,
    },
    device::HeapId,
    error::{Error, Result},
    heap::{HeapDescriptor, PAGE},
};

/// A heap the case made.
pub struct Heap {
    pub name: String,
    pub id: HeapId,
    /// Where the CPU reaches a shared heap's memory.
    pub pointer: Option<NonNull<u8>>,
    pub options: ResourceOptions,
    pub placement: bool,
}

/// What a heap may say.
pub const HEAP: [&str; 5] = ["name", "size", "storage", "type", "untracked"];

impl Runner<'_> {
    /// Makes the case's heaps into `held`.
    pub(crate) fn make_heaps(&mut self, specs: &Value, held: &mut Held) -> Result<()> {
        for spec in specs.as_array().into_iter().flatten() {
            let placement = spec["type"] == "placement";
            let shared = spec["storage"] != "private";
            let options = ResourceOptions {
                cpu_cache_mode: CpuCacheMode::DefaultCache,
                storage_mode: if shared {
                    StorageMode::Shared
                } else {
                    StorageMode::Private
                },
                hazard_tracking_mode: if spec["untracked"] == true {
                    HazardTrackingMode::Untracked
                } else {
                    HazardTrackingMode::Tracked
                },
            };
            let descriptor = HeapDescriptor {
                size: spec["size"].as_u64().unwrap_or(65536),
                options,
                heap_type: if placement {
                    HeapType::Placement
                } else {
                    HeapType::Automatic
                },
                sparse_page_size: SparsePageSize::Size16,
            };
            let (id, pointer) = if shared {
                // A Mac holds a heap in whole pages.
                let bytes = descriptor.size.next_multiple_of(PAGE);
                let index = self.next_import;
                self.next_import += 1;
                let (memory, pointer) =
                    super::device::shared_memory(self.device, self.exporter, index, bytes)?;
                (
                    self.device.new_imported_heap(memory, &descriptor)?,
                    Some(pointer),
                )
            } else {
                (self.device.new_heap(&descriptor)?, None)
            };
            held.heaps.push(Heap {
                name: spec["name"].as_str().unwrap_or_default().to_owned(),
                id,
                pointer,
                options,
                placement,
            });
        }
        Ok(())
    }

    /// A buffer of `length` bytes in the heap `spec` names, holding `data`
    /// unless it aliases another resource, whose bytes it keeps.
    pub(crate) fn heap_buffer(
        &mut self,
        spec: &Value,
        data: &[u8],
        length: u64,
        held: &Held,
    ) -> Result<Shared> {
        let name = spec["heap"].as_str().unwrap_or_default();
        let heap = held
            .heaps
            .iter()
            .find(|heap| heap.name == name)
            .ok_or(Error::Invalid("a buffer names no heap"))?;
        let pointer = heap
            .pointer
            .ok_or(Error::Unsupported("a buffer in a private heap"))?;
        let length = length.max(data.len() as u64).max(16);
        let offset = heap
            .placement
            .then(|| spec["heap_offset"].as_u64().unwrap_or(0));
        let id = self
            .device
            .new_heap_buffer(heap.id, length, heap.options, offset)?;
        let at = self.device.buffer_placement(id)?.heap_offset;
        // SAFETY: the heap's mapping covers its every placement.
        let pointer = unsafe { NonNull::new_unchecked(pointer.as_ptr().add(at as usize)) };
        if spec["alias_of"].is_null() {
            // SAFETY: the buffer spans `length` bytes of the mapping.
            unsafe {
                core::ptr::write_bytes(pointer.as_ptr(), 0, length as usize);
                core::ptr::copy_nonoverlapping(data.as_ptr(), pointer.as_ptr(), data.len());
            }
        }
        Ok(Shared {
            id,
            pointer,
            length,
        })
    }
}
