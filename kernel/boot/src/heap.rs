//! Boot-stage heap glue (M2.5): wires the `arena-heap` core to the frame
//! allocator and publishes the kernel-facing API. All policy lives in the
//! host-testable crate; this file is only the provider + the static
//! instance + error logging (ADR-0009).

use crate::frames;
use crate::log::{log_error as error, log_info as info};
use crate::sync::SyncCell;
use arena_heap::{CHUNK_BYTES_DEFAULT, ChunkProvider, Heap, HeapError};
use core::alloc::Layout;
use core::ptr::NonNull;

/// Chunk source: physically contiguous frame runs (16 × 4 KiB for the
/// default 64 KiB chunk) from the M2.3 allocator. Chunks are conventional
/// memory, identity-mapped RW+NX like every other frame.
pub struct FrameChunkProvider;

impl ChunkProvider for FrameChunkProvider {
    fn provide_chunk(&mut self, bytes: usize) -> Option<NonNull<u8>> {
        let needed = bytes.div_ceil(4096);
        let phys = frames::alloc_contiguous(needed)?;
        NonNull::new(phys as *mut u8)
    }
}

/// The single boot-stage heap. Guards are ALWAYS on here: the overhead is
/// irrelevant at boot scale and catching corruption at the free that
/// caused it (instead of three subsystems later) is the entire point of
/// this milestone. No locking — boot contract (single CPU, IF=0); the
/// kernel proper wraps this in the M2.6 primitives.
static HEAP: SyncCell<Heap<FrameChunkProvider>> =
    SyncCell::new(Heap::new(FrameChunkProvider, true, CHUNK_BYTES_DEFAULT));

/// Allocate raw bytes per `layout` (None on zero-size, over-chunk, or
/// frame exhaustion). Payloads arrive poisoned 0xAA and red-zoned.
pub fn alloc(layout: Layout) -> Option<NonNull<u8>> {
    // SAFETY: boot contract — single CPU, IF=0; SyncCell grants exclusive
    // access; the heap upholds its own invariants (host-tested core).
    unsafe { (*HEAP.get()).alloc(layout) }
}

/// Typed convenience over [`alloc`] (size/align from `T`).
pub fn alloc_typed<T>() -> Option<NonNull<T>> {
    // SAFETY: as `alloc`.
    unsafe { (*HEAP.get()).alloc_typed::<T>() }
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
    let result = unsafe { (*HEAP.get()).free(ptr) };
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
    // SAFETY: boot contract; plain stat reads.
    unsafe { (*HEAP.get()).in_use_bytes() }
}
pub fn blocks_live() -> usize {
    // SAFETY: boot contract.
    unsafe { (*HEAP.get()).blocks_live() }
}
pub fn bytes_reserved() -> usize {
    // SAFETY: boot contract.
    unsafe { (*HEAP.get()).bytes_reserved() }
}
pub fn chunk_count() -> usize {
    // SAFETY: boot contract.
    unsafe { (*HEAP.get()).chunk_count() }
}
pub fn alloc_count() -> usize {
    // SAFETY: boot contract.
    unsafe { (*HEAP.get()).alloc_count() }
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
