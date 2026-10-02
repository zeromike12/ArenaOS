//! Boot-stage heap glue (M2.5, lock-wrapped at M2.6): wires the
//! `arena-heap` core to the frame allocator and publishes the
//! kernel-facing API. All policy lives in the host-testable crates; this
//! file is only the provider + the static instance + error logging
//! (ADR-0009, ADR-0010).

use crate::frames;
use crate::log::{log_error as error, log_info as info};
use arena_heap::{CHUNK_BYTES_DEFAULT, ChunkProvider, Heap, HeapError};
use arena_sync::Spinlock;
use core::alloc::Layout;
use core::ptr::NonNull;

/// Chunk source: physically contiguous frame runs (16 × 4 KiB for the
/// default 64 KiB chunk) from the M2.3 allocator. Chunks are conventional
/// memory, mapped RW+NX in whichever view is live: the boot stage runs
/// them at identity addresses (`va_offset` 0); after the M2.7 kernel-view
/// switch the re-initialized heap requests them at their direct-map
/// aliases (`va_offset` = KERNEL_OFFSET).
pub struct FrameChunkProvider {
    pub va_offset: u64,
}

impl ChunkProvider for FrameChunkProvider {
    fn provide_chunk(&mut self, bytes: usize) -> Option<NonNull<u8>> {
        let needed = bytes.div_ceil(4096);
        let phys = frames::alloc_contiguous(needed)?;
        NonNull::new((phys + self.va_offset) as *mut u8)
    }
}

/// Boot-stage CPU token: the BSP is the only CPU that exists before
/// ExitBootServices (ADR-0003). Owner tracking uses it to catch recursive
/// acquisition in debug builds; SMP supplies real ids later (ADR-0010).
fn boot_cpu_id() -> u32 {
    0
}

/// The single boot-stage heap. Guards are ALWAYS on here: the overhead is
/// irrelevant at boot scale and catching corruption at the free that
/// caused it (instead of three subsystems later) is the entire point of
/// M2.5. Since M2.6 every access goes through the `arena-sync` spinlock —
/// a no-op for contention on the single boot CPU, but it discharges the
/// ADR-0009 promise, makes the boot code the same shape as the kernel
/// proper, and its guard/owner machinery is host-tested under real
/// parallelism.
static HEAP: Spinlock<Heap<FrameChunkProvider>> = Spinlock::new(
    Heap::new(
        FrameChunkProvider { va_offset: 0 },
        true,
        CHUNK_BYTES_DEFAULT,
    ),
    boot_cpu_id,
);

/// Allocate raw bytes per `layout` (None on zero-size, over-chunk, or
/// frame exhaustion). Payloads arrive poisoned 0xAA and red-zoned.
pub fn alloc(layout: Layout) -> Option<NonNull<u8>> {
    HEAP.lock().alloc(layout)
}

/// Typed convenience over [`alloc`] (size/align from `T`).
pub fn alloc_typed<T>() -> Option<NonNull<T>> {
    HEAP.lock().alloc_typed::<T>()
}

/// Release a pointer from [`alloc`]. Rejections (double free, red-zone
/// violations, foreign pointers) are logged and returned; the block stays
/// live on rejection.
///
/// # Safety
/// `ptr` must be exactly what [`alloc`] returned (or a deliberate
/// bad pointer when exercising the rejection paths).
pub unsafe fn free(ptr: NonNull<u8>) -> Result<(), HeapError> {
    // SAFETY: caller contract forwarded to the heap's free contract.
    // The spinlock guard is held for the whole operation.
    let result = unsafe { HEAP.lock().free(ptr) };
    if let Err(reason) = result {
        error!("heap", "free rejected: {reason:?} ptr={:p}", ptr.as_ptr());
    }
    result
}

/// Typed convenience over [`free`].
///
/// # Safety
/// As [`free`]; `ptr` must come from [`alloc_typed::<T>`].
pub unsafe fn free_typed<T>(ptr: NonNull<T>) -> Result<(), HeapError> {
    // SAFETY: caller contract forwarded (same pointer, byte view).
    unsafe { free(ptr.cast::<u8>()) }
}

pub fn in_use_bytes() -> usize {
    HEAP.lock().in_use_bytes()
}
pub fn blocks_live() -> usize {
    HEAP.lock().blocks_live()
}
pub fn bytes_reserved() -> usize {
    HEAP.lock().bytes_reserved()
}
pub fn chunk_count() -> usize {
    HEAP.lock().chunk_count()
}
pub fn alloc_count() -> usize {
    HEAP.lock().alloc_count()
}

/// M2.7 kernel-view switch: the boot-stage heap must be EMPTY here (the
/// test suites free everything — asserted), its identity-addressed chunks
/// are released back to the frame allocator, and a pristine heap is
/// installed whose provider hands out direct-map (higher-half) addresses.
/// Returns the number of chunks released.
pub fn reinit_for_kernel_view() -> Result<u64, &'static str> {
    let mut heap = HEAP.lock();
    if heap.in_use_bytes() != 0 || heap.blocks_live() != 0 {
        return Err("heap not empty at kernel-view switch");
    }
    let mut released = 0u64;
    // SAFETY: emptiness checked above; single CPU, IF=0; every existing
    // chunk was provided with va_offset 0, so its base pointer IS its
    // physical address — exactly what free_contiguous wants.
    unsafe {
        heap.release_chunks(|ptr, bytes| {
            let frames_n = bytes / 4096;
            if let Err(reason) = frames::free_contiguous(ptr.as_ptr() as u64, frames_n) {
                error!(
                    "heap",
                    "chunk release failed ({reason}) — leaking {frames_n} frames"
                );
            }
            released += 1;
        });
    }
    *heap = Heap::new(
        FrameChunkProvider {
            va_offset: crate::arch::x86_64::paging::KERNEL_OFFSET,
        },
        true,
        CHUNK_BYTES_DEFAULT,
    );
    Ok(released)
}

/// Boot step 3.8: the heap is lazy (first allocation grows the first
/// chunk), so init only announces the configuration. Failure is impossible
/// here by construction; frame exhaustion surfaces later as `None` from
/// `alloc`, which callers must handle anyway.
pub fn init() {
    info!(
        "heap",
        "kernel heap ready: guards=on chunk={}KiB frame-backed (arena-heap core, host-tested)",
        CHUNK_BYTES_DEFAULT / 1024
    );
}
