//! ArenaOS kernel heap (M2.5, ADR-0009) — the allocator core.
//!
//! Design: a first-fit free list over fixed-size *chunks* obtained from a
//! [`ChunkProvider`] (the boot stage wires it to the physical frame
//! allocator; host-side tests wire it to a `Vec`). Blocks carry a 32-byte
//! header; free blocks link through the header (payloads stay untouched, so
//! free-poison is fully observable). The free list is address-ordered and
//! frees coalesce with both neighbors, so long runs return to single
//! blocks. Chunks are never returned to the provider (boot scale; the
//! kernel proper revisits shrink paths with real workloads).
//!
//! Debug guards (enabled by the consumer, always-on in the boot stage):
//! * red zone: a magic word in the header (front) and an unaligned magic
//!   word directly after the user bytes (back), checked on `free`;
//! * poisoning: freed payloads are filled `0xDD`, freshly allocated
//!   payloads `0xAA` — no consumer ever sees stale or uninitialized-looking
//!   data by accident;
//! * `free` validates the pointer by walking chunk block chains: wild
//!   pointers and interior pointers are rejected `NotFromHeap` instead of
//!   corrupting metadata.
//!
//! Alignment: every payload is at least [`HEAP_ALIGN`]-byte aligned; larger
//! alignments are satisfied by shifting the header inside the candidate
//! block and splitting the front slack off as its own free block.
//!
//! Concurrency: none — `Heap` is a plain value owned by its environment.
//! The boot stage holds one in a `SyncCell` under the single-CPU/IF=0 boot
//! contract; the kernel proper wraps it in the M2.6 lock primitives.
//!
//! This crate is `no_std` except under `cfg(test)`, where the std test
//! harness runs the host-side unit tests required by ROADMAP 2.5.

#![cfg_attr(not(test), no_std)]

use core::alloc::Layout;
use core::ptr::NonNull;

/// Alignment guaranteed for every payload, and the granularity of all
/// block footprints (header included).
pub const HEAP_ALIGN: usize = 16;

/// Default chunk size requested from the provider (16 frames at 4 KiB).
pub const CHUNK_BYTES_DEFAULT: usize = 64 * 1024;

const HEADER_SIZE: usize = 32;
const FREE_FLAG: u64 = 1;
/// Footprint occupies the high bits; the low 4 are flags (footprints are
/// multiples of HEAP_ALIGN = 16).
const FOOTPRINT_MASK: u64 = !0xF;
/// Smallest footprint a free block may have: the header alone (the free
/// link lives in the header, not the payload).
const MIN_FREE_FOOTPRINT: usize = HEADER_SIZE;
/// Smallest payload slot for a live block (keeps footprints sane for tiny
/// and zero-guard requests).
const MIN_PAYLOAD_SLOT: usize = 16;

const CHUNK_MAGIC: u64 = 0x4152_4E48_4348_4B31; // "ARNHCHK1"
const RED_MAGIC: u64 = 0xC33C_DEAD_BEEF_5AA5;
const ALLOC_FILL: u8 = 0xAA;
const FREE_FILL: u8 = 0xDD;

/// Why a `free` was rejected. Rejections never mutate the block: a failed
/// free leaves the allocation live (the consumer decides — the boot glue
/// logs loudly).
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum HeapError {
    /// The block's FREE flag was already set.
    DoubleFree,
    /// Header red-zone word was not `RED_MAGIC` (underflow write).
    RedZoneFront,
    /// Trailing red-zone word was not `RED_MAGIC` (overflow write).
    RedZoneBack,
    /// The pointer is not the payload start of a live block in any chunk
    /// (wild, interior, or foreign pointer).
    NotFromHeap,
}

/// Block header — the first 32 bytes of every block, live or free.
#[repr(C)]
struct Header {
    /// Block footprint (header + payload slot, multiple of HEAP_ALIGN) in
    /// the high bits, [`FREE_FLAG`] in bit 0.
    footprint_flags: u64,
    /// Exact user-requested size (meaningful while live; retained while
    /// free for diagnostics).
    user_size: u64,
    /// Front red zone ([`RED_MAGIC`] while live with guards on).
    red_front: u64,
    /// Next free block's header address (address-ordered list, 0 = end).
    /// Only meaningful while FREE — deliberately NOT in the payload, so
    /// free-poison covers every user byte.
    free_link: u64,
}
const _: () = assert!(core::mem::size_of::<Header>() == 32);

impl Header {
    fn footprint(&self) -> usize {
        (self.footprint_flags & FOOTPRINT_MASK) as usize
    }
    fn is_free(&self) -> bool {
        self.footprint_flags & FREE_FLAG != 0
    }
    fn init(&mut self, footprint: usize, free: bool) {
        debug_assert!(footprint >= HEADER_SIZE && footprint.is_multiple_of(HEAP_ALIGN));
        self.footprint_flags = footprint as u64 | u64::from(free);
        self.user_size = 0;
        self.red_front = 0;
        self.free_link = 0;
    }
    fn set_free_flag(&mut self, free: bool) {
        self.footprint_flags = (self.footprint_flags & FOOTPRINT_MASK) | u64::from(free);
    }
    fn end_addr(&self) -> usize {
        self as *const Self as usize + self.footprint()
    }
    /// Payload start (address handed to the user).
    fn payload(&self) -> usize {
        self as *const Self as usize + HEADER_SIZE
    }
}

/// Chunk prologue. Chunks are linked newest-first; blocks never span
/// chunks (the 32-byte prologue keeps adjacent chunks' blocks from
/// coalescing across the boundary).
#[repr(C)]
struct ChunkHeader {
    magic: u64,
    chunk_bytes: u64,
    /// Next chunk header address (0 = end).
    next: u64,
    _pad: u64,
}
const _: () = assert!(core::mem::size_of::<ChunkHeader>() == 32);

/// Supplies raw chunk memory. Contract: return a region of exactly
/// `bytes` (a multiple of 4 KiB in practice), aligned to at least
/// [`HEAP_ALIGN`], not aliased by anything else; the heap owns it forever.
pub trait ChunkProvider {
    fn provide_chunk(&mut self, bytes: usize) -> Option<NonNull<u8>>;
}

/// The heap. See the crate docs for the design; field docs carry the
/// invariants.
pub struct Heap<P: ChunkProvider> {
    provider: P,
    /// Head of the address-ordered free list (header addresses, 0 = empty).
    free_head: usize,
    /// Head of the chunk list (chunk header addresses, 0 = none).
    chunk_head: usize,
    chunk_bytes: usize,
    guards: bool,
    // --- accounting (asserted by tests, logged by boot) ---
    /// Sum of exact user sizes of live allocations.
    bytes_in_use: usize,
    /// Number of live allocations.
    blocks_live: usize,
    /// Total bytes obtained from the provider.
    bytes_reserved: usize,
    /// Chunks obtained from the provider.
    chunk_count: usize,
    /// Total successful allocations over the heap's life.
    alloc_count: u64,
}

impl<P: ChunkProvider> Heap<P> {
    /// A heap with no chunks; the first allocation grows it. `const` so
    /// consumers can hold it in a static.
    pub const fn new(provider: P, guards: bool, chunk_bytes: usize) -> Self {
        // Chunks are carved into 16-byte-aligned blocks; anything smaller
        // or unaligned would break the footprint encoding.
        assert!(chunk_bytes.is_multiple_of(4096) && chunk_bytes >= 4096);
        Self {
            provider,
            free_head: 0,
            chunk_head: 0,
            chunk_bytes,
            guards,
            bytes_in_use: 0,
            blocks_live: 0,
            bytes_reserved: 0,
            chunk_count: 0,
            alloc_count: 0,
        }
    }

    pub fn in_use_bytes(&self) -> usize {
        self.bytes_in_use
    }
    pub fn blocks_live(&self) -> usize {
        self.blocks_live
    }
    pub fn bytes_reserved(&self) -> usize {
        self.bytes_reserved
    }
    pub fn chunk_count(&self) -> usize {
        self.chunk_count
    }
    pub fn alloc_count(&self) -> usize {
        self.alloc_count as usize
    }
    pub fn guards_enabled(&self) -> bool {
        self.guards
    }

    /// Hand every chunk back to the caller (M2.7 kernel-view switch: the
    /// boot-stage chunks were obtained under identity-mapped addresses and
    /// the kernel re-requests memory under its own view). The heap must be
    /// empty — asserted — and returns to the pristine state of
    /// [`Heap::new`], except that `alloc_count` (a lifetime statistic) is
    /// preserved. `f` receives each chunk base exactly as the provider
    /// returned it, plus its size in bytes, newest chunk first, and must
    /// release the underlying memory.
    ///
    /// # Safety
    /// No pointer into any chunk may be used after this call (an empty
    /// heap is necessary but not sufficient — dangling user pointers are
    /// the caller's problem), and the chunk memory `f` releases must not
    /// alias anything still in use.
    pub unsafe fn release_chunks(&mut self, mut f: impl FnMut(NonNull<u8>, usize)) {
        assert_eq!(self.bytes_in_use, 0, "release_chunks with live allocations");
        assert_eq!(self.blocks_live, 0, "release_chunks with live blocks");
        let mut cur = self.chunk_head;
        while cur != 0 {
            // SAFETY: `cur` is a chunk header this heap wrote in `grow()`;
            // the caller guarantees no other accessor, and `&mut self` is
            // held for the walk.
            let (bytes, next) = unsafe {
                let ch = cur as *const ChunkHeader;
                debug_assert_eq!((*ch).magic, CHUNK_MAGIC);
                ((*ch).chunk_bytes as usize, (*ch).next as usize)
            };
            let Some(ptr) = NonNull::new(cur as *mut u8) else {
                break;
            };
            f(ptr, bytes);
            cur = next;
        }
        self.free_head = 0;
        self.chunk_head = 0;
        self.bytes_reserved = 0;
        self.chunk_count = 0;
    }

    /// Allocate `layout.size()` bytes with `layout.align()` alignment.
    ///
    /// Returns `None` for zero-sized layouts, layouts needing more than a
    /// whole chunk, or provider exhaustion. Payloads are poisoned
    /// `ALLOC_FILL` and (guards on) red-zoned; the user size is recorded
    /// exactly.
    pub fn alloc(&mut self, layout: Layout) -> Option<NonNull<u8>> {
        if layout.size() == 0 {
            return None;
        }
        if layout.size() + HEADER_SIZE + usize::from(self.guards) * 8 + HEAP_ALIGN
            > self.chunk_bytes - HEADER_SIZE
        {
            // Could never fit in a fresh chunk; refuse rather than spin.
            return None;
        }
        // Try the free list; on failure grow once and retry.
        let align = if layout.align() > HEAP_ALIGN {
            layout.align()
        } else {
            HEAP_ALIGN
        };
        if let Some(p) = self.alloc_from_freelist(layout.size(), align) {
            return Some(p);
        }
        if !self.grow() {
            return None;
        }
        self.alloc_from_freelist(layout.size(), align)
    }

    /// Typed convenience: allocate for `T` (size + align from its layout).
    pub fn alloc_typed<T>(&mut self) -> Option<NonNull<T>> {
        self.alloc(Layout::new::<T>()).map(|p| p.cast::<T>())
    }

    /// Release a pointer previously returned by [`Heap::alloc`].
    ///
    /// Rejects (without mutating anything): double frees, red-zone
    /// violations (guards on), and pointers that are not the payload start
    /// of a live block. On success the payload is poisoned `FREE_FILL` and
    /// the block coalesces with free neighbors.
    ///
    /// # Safety
    /// `ptr` must be the exact payload pointer returned by a previous
    /// successful `alloc` on this heap, or a value the caller obtained
    /// *to test rejection paths* — rejections are memory-safe, but
    /// freeing a live allocation while other references to it exist is a
    /// use-after-free the heap cannot detect.
    pub unsafe fn free(&mut self, ptr: NonNull<u8>) -> Result<(), HeapError> {
        let raw = ptr.as_ptr() as usize;
        if raw < HEADER_SIZE {
            return Err(HeapError::NotFromHeap);
        }
        let target = raw - HEADER_SIZE;
        // SAFETY: find_block validates `target` is a real block header in
        // a chunk we own before we dereference it here.
        let Some(h_addr) = self.find_block(target) else {
            return Err(HeapError::NotFromHeap);
        };
        unsafe {
            let h = h_addr as *mut Header;
            if (*h).is_free() {
                return Err(HeapError::DoubleFree);
            }
            let user_size = (*h).user_size as usize;
            let payload = (*h).payload();
            if self.guards {
                if (*h).red_front != RED_MAGIC {
                    return Err(HeapError::RedZoneFront);
                }
                let back = (payload + user_size) as *const u64;
                if back.read_unaligned() != RED_MAGIC {
                    return Err(HeapError::RedZoneBack);
                }
            }
            core::ptr::write_bytes(payload as *mut u8, FREE_FILL, user_size);
            (*h).set_free_flag(true);
            (*h).red_front = 0;
            self.bytes_in_use -= user_size;
            self.blocks_live -= 1;
            self.insert_free_coalescing(h_addr);
        }
        Ok(())
    }

    /// Typed convenience for [`Heap::free`].
    ///
    /// # Safety
    /// As [`Heap::free`]; `ptr` must have come from `alloc_typed::<T>`.
    pub unsafe fn free_typed<T>(&mut self, ptr: NonNull<T>) -> Result<(), HeapError> {
        // SAFETY: forwarded to caller contract (same pointer, byte view).
        unsafe { self.free(ptr.cast::<u8>()) }
    }

    // ------------------------------------------------------------------
    // internals
    // ------------------------------------------------------------------

    /// Footprint of the payload slot for a request: user bytes + trailing
    /// red-zone word (guards on), rounded up to alignment granularity.
    fn payload_slot(&self, user_size: usize) -> usize {
        let raw = user_size + usize::from(self.guards) * 8;
        align_up(raw, HEAP_ALIGN).max(MIN_PAYLOAD_SLOT)
    }

    /// Obtain one chunk from the provider and seed it with a single free
    /// block covering everything behind the chunk prologue.
    fn grow(&mut self) -> bool {
        let Some(chunk) = self.provider.provide_chunk(self.chunk_bytes) else {
            return false;
        };
        let base = chunk.as_ptr() as usize;
        // SAFETY: the provider contract grants exclusive ownership of an
        // aligned chunk_bytes region; we write only inside it.
        unsafe {
            let ch = base as *mut ChunkHeader;
            (*ch).magic = CHUNK_MAGIC;
            (*ch).chunk_bytes = self.chunk_bytes as u64;
            (*ch).next = self.chunk_head as u64;
            (*ch)._pad = 0;
            self.chunk_head = base;

            let first = base + HEADER_SIZE;
            let h = first as *mut Header;
            (*h).init(self.chunk_bytes - HEADER_SIZE, true);
            self.bytes_reserved += self.chunk_bytes;
            self.chunk_count += 1;
        }
        self.insert_free_coalescing(base + HEADER_SIZE);
        true
    }

    /// First-fit scan of the address-ordered free list. `align` is
    /// `HEAP_ALIGN` or larger (a power of two).
    fn alloc_from_freelist(&mut self, size: usize, align: usize) -> Option<NonNull<u8>> {
        let need = HEADER_SIZE + self.payload_slot(size);
        // `prev_link` is the ADDRESS of the link field pointing at `cur`
        // (either `&mut self.free_head` or a predecessor's `free_link`).
        let mut prev_link: *mut usize = &mut self.free_head;
        let mut cur = self.free_head;
        while cur != 0 {
            // SAFETY: free-list nodes are live Header structs inside our
            // chunks; the list is walked under exclusive &mut self.
            let (block_foot, next) = unsafe {
                let h = cur as *const Header;
                ((*h).footprint(), (*h).free_link as usize)
            };

            // Where must the header sit for `align`?
            let payload0 = cur + HEADER_SIZE;
            let hpos = if align > HEAP_ALIGN {
                align_up(payload0, align) - HEADER_SIZE
            } else {
                cur
            };
            let slack = hpos - cur;
            // A leftover tail smaller than MIN_FREE_FOOTPRINT is absorbed
            // into the allocated block (internal padding), so only the
            // front slack has a minimum.
            let usable = block_foot >= slack + need && (slack == 0 || slack >= MIN_FREE_FOOTPRINT);

            if usable {
                // Unlink `cur`; front slack and tail remainder are split
                // off as free blocks (re-inserted sorted, coalescing).
                unsafe {
                    *prev_link = next;
                    let h = hpos as *mut Header;
                    let mut footprint = block_foot - slack;
                    let tail = footprint - need;
                    if tail >= MIN_FREE_FOOTPRINT {
                        footprint = need;
                        let t = hpos + need;
                        let th = t as *mut Header;
                        (*th).init(tail, true);
                        self.insert_free_coalescing(t);
                    }
                    if slack >= MIN_FREE_FOOTPRINT {
                        let fh = cur as *mut Header;
                        (*fh).init(slack, true);
                        self.insert_free_coalescing(cur);
                    }
                    (*h).init(footprint, false);
                    (*h).user_size = size as u64;
                    let payload = (*h).payload();
                    if self.guards {
                        (*h).red_front = RED_MAGIC;
                        ((payload + size) as *mut u64).write_unaligned(RED_MAGIC);
                        core::ptr::write_bytes(payload as *mut u8, ALLOC_FILL, size);
                    }
                    self.bytes_in_use += size;
                    self.blocks_live += 1;
                    self.alloc_count += 1;
                    return NonNull::new(payload as *mut u8);
                }
            }

            prev_link =
                unsafe { core::ptr::addr_of_mut!((*(cur as *mut Header)).free_link) as *mut usize };
            cur = next;
        }
        None
    }

    /// Insert a freshly freed block at its address-ordered position,
    /// coalescing with the next and/or previous free block when adjacent.
    fn insert_free_coalescing(&mut self, block: usize) {
        let mut prev_link: *mut usize = &mut self.free_head;
        let mut cur = self.free_head;
        // SAFETY: as alloc_from_freelist; `block` is a header we just
        // initialized FREE and is not in the list.
        unsafe {
            let bh = block as *mut Header;
            let block_end = (*bh).end_addr();
            while cur != 0 && cur < block {
                let h = cur as *mut Header;
                if (*h).end_addr() == block {
                    // Previous free block is adjacent: absorb `block`.
                    // Preserve the list link — init() would clear it.
                    let grown = (*h).footprint() + (*bh).footprint();
                    let link = (*h).free_link;
                    (*h).init(grown, true);
                    (*h).free_link = link;
                    return;
                }
                prev_link = &mut (*h).free_link as *mut u64 as *mut usize;
                cur = (*h).free_link as usize;
            }
            // `cur` is the first free block after `block` (or 0).
            if cur != 0 && block_end == cur {
                // Absorb the successor into `block`.
                let ch = cur as *mut Header;
                let grown = (*bh).footprint() + (*ch).footprint();
                (*bh).init(grown, true);
                (*bh).free_link = (*ch).free_link;
            } else {
                (*bh).free_link = cur as u64;
            }
            *prev_link = block;
        }
    }

    /// Validate that `target` is exactly the address of a block header in
    /// one of our chunks, walking the chunk's block chain. Returns the
    /// header address (== `target`) or `None`.
    fn find_block(&self, target: usize) -> Option<usize> {
        let mut chunk = self.chunk_head;
        while chunk != 0 {
            // SAFETY: chunk list nodes are ChunkHeaders we wrote in grow();
            // read-only walk under &self.
            unsafe {
                let ch = chunk as *const ChunkHeader;
                if (*ch).magic != CHUNK_MAGIC {
                    return None; // metadata corruption — refuse everything
                }
                let chunk_end = chunk + (*ch).chunk_bytes as usize;
                if target >= chunk + HEADER_SIZE && target + HEADER_SIZE <= chunk_end {
                    let mut b = chunk + HEADER_SIZE;
                    while b + HEADER_SIZE <= chunk_end {
                        if b == target {
                            return Some(b);
                        }
                        let foot = (*(b as *const Header)).footprint();
                        if foot == 0 || !foot.is_multiple_of(HEAP_ALIGN) {
                            return None; // chain corruption
                        }
                        b += foot;
                    }
                    return None; // walked past: interior/foreign pointer
                }
                chunk = (*ch).next as usize;
            }
        }
        None
    }
}

fn align_up(v: usize, a: usize) -> usize {
    v.div_ceil(a) * a
}

// ===========================================================================
// Host-side unit tests (ROADMAP 2.5): the allocator logic runs unchanged on
// the host against a Vec-backed provider.
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::alloc::{GlobalAlloc, Layout as StdLayout, System};
    use std::collections::BTreeMap;
    use std::vec::Vec;

    /// Provider handing out 4 KiB-aligned chunks from the host allocator,
    /// capped at `limit` chunks (finite, so exhaustion is testable).
    struct TestProvider {
        chunk_bytes: usize,
        limit: usize,
        handed: usize,
    }

    impl ChunkProvider for TestProvider {
        fn provide_chunk(&mut self, bytes: usize) -> Option<NonNull<u8>> {
            assert_eq!(bytes, self.chunk_bytes);
            if self.handed >= self.limit {
                return None;
            }
            self.handed += 1;
            // SAFETY: host test allocator; layout is valid (nonzero size,
            // power-of-two align); the pointer is stored implicitly by the
            // heap (chunks are never returned — leaked by design here).
            unsafe {
                let p = System.alloc(StdLayout::from_size_align(bytes, 4096).unwrap());
                NonNull::new(p)
            }
        }
    }

    fn heap(guards: bool, chunks: usize) -> Heap<TestProvider> {
        Heap::new(
            TestProvider {
                chunk_bytes: 4096,
                limit: chunks,
                handed: 0,
            },
            guards,
            4096,
        )
    }

    fn layout(size: usize) -> Layout {
        Layout::from_size_align(size, HEAP_ALIGN).unwrap()
    }

    /// Run the core behavior suite in both guard modes.
    fn core_suite(guards: bool) {
        let mut h = heap(guards, 4);
        let a = h.alloc(layout(64)).expect("a");
        let b = h.alloc(layout(128)).expect("b");
        assert_eq!(h.blocks_live(), 2);
        assert_eq!(h.in_use_bytes(), 192);
        // Payloads are distinct, aligned, and inside a chunk.
        assert_ne!(a.as_ptr(), b.as_ptr());
        assert_eq!(a.as_ptr() as usize % HEAP_ALIGN, 0);
        assert_eq!(b.as_ptr() as usize % HEAP_ALIGN, 0);
        // Write patterns, verify round-trip.
        unsafe {
            core::ptr::write_bytes(a.as_ptr(), 0x11, 64);
            core::ptr::write_bytes(b.as_ptr(), 0x22, 128);
            assert_eq!(*a.as_ptr(), 0x11);
            assert_eq!(*b.as_ptr().add(127), 0x22);
        }
        // Guards on: fresh payload was poisoned 0xAA before we wrote it —
        // covered implicitly; verify free poisoning below.
        unsafe { h.free(a).expect("free a") };
        if guards {
            // Freed payload is poisoned 0xDD.
            unsafe { assert_eq!(*a.as_ptr(), 0xDD) };
        }
        assert_eq!(h.blocks_live(), 1);
        assert_eq!(h.in_use_bytes(), 128);
        // Realloc same size reuses the freed block (first-fit).
        let a2 = h.alloc(layout(64)).expect("a2");
        assert_eq!(a2.as_ptr(), a.as_ptr());
        unsafe {
            h.free(a2).expect("free a2");
            h.free(b).expect("free b");
        }
        assert_eq!(h.blocks_live(), 0);
        assert_eq!(h.in_use_bytes(), 0);
        // Full coalescing: the whole first chunk (minus prologue+header)
        // is one block again — a near-chunk allocation must fit.
        let big = h
            .alloc(layout(4096 - HEADER_SIZE - HEADER_SIZE - 8 - HEAP_ALIGN))
            .expect("near-chunk alloc after coalescing");
        unsafe { h.free(big).expect("free big") };
    }

    #[test]
    fn core_guards_on() {
        core_suite(true);
    }

    #[test]
    fn core_guards_off() {
        core_suite(false);
    }

    #[test]
    fn zero_size_and_oversize_rejected() {
        let mut h = heap(true, 2);
        assert!(h.alloc(layout(0)).is_none());
        assert!(
            h.alloc(Layout::from_size_align(8192, 16).unwrap())
                .is_none()
        );
        assert_eq!(h.blocks_live(), 0);
    }

    #[test]
    fn alignment_requests() {
        let mut h = heap(true, 4);
        // 4096 alignment is unsatisfiable inside 4 KiB test chunks (the
        // payload could only land on the next chunk's boundary) — covered
        // by the in-guest test at 64 KiB chunks instead.
        for align in [16usize, 32, 64, 128, 256, 512] {
            for size in [1usize, 8, 33, 100, 512] {
                let l = Layout::from_size_align(size, align).unwrap();
                let p = h.alloc(l).expect("aligned alloc");
                assert_eq!(
                    p.as_ptr() as usize % align,
                    0,
                    "align {align} size {size} gave {:?}",
                    p.as_ptr()
                );
                unsafe {
                    core::ptr::write_bytes(p.as_ptr(), 0x5A, size);
                    h.free(p).expect("aligned free");
                }
            }
        }
        assert_eq!(h.blocks_live(), 0);
    }

    #[test]
    fn double_free_rejected() {
        let mut h = heap(true, 2);
        let a = h.alloc(layout(48)).unwrap();
        unsafe {
            assert_eq!(h.free(a), Ok(()));
            assert_eq!(h.free(a), Err(HeapError::DoubleFree));
        }
        // Heap still functional.
        let b = h.alloc(layout(48)).unwrap();
        assert_eq!(b.as_ptr(), a.as_ptr());
        unsafe { h.free(b).unwrap() };
    }

    #[test]
    fn red_zones_catch_overflow_and_underflow() {
        let mut h = heap(true, 2);
        // Overflow: write into the trailing red zone.
        let a = h.alloc(layout(32)).unwrap();
        unsafe {
            *a.as_ptr().add(32) = 0x99; // clobbers RED_MAGIC's first byte
            assert_eq!(h.free(a), Err(HeapError::RedZoneBack));
        }
        assert_eq!(h.blocks_live(), 1, "rejected free must keep block live");
        // Underflow: write before the payload (into the header's red word).
        let b = h.alloc(layout(32)).unwrap();
        unsafe {
            *b.as_ptr().sub(16) = 0x77; // red_front's low byte
            assert_eq!(h.free(b), Err(HeapError::RedZoneFront));
        }
        // Heap still allocates fine afterwards.
        let c = h.alloc(layout(32));
        assert!(c.is_some());
    }

    #[test]
    fn foreign_and_interior_pointers_rejected() {
        let mut h = heap(true, 2);
        let a = h.alloc(layout(64)).unwrap();
        let stack_var = 0u64;
        let foreign = NonNull::new(&stack_var as *const u64 as *mut u8).unwrap();
        unsafe {
            assert_eq!(h.free(foreign), Err(HeapError::NotFromHeap));
            // Interior pointer of a live block.
            let interior = NonNull::new_unchecked(a.as_ptr().add(16));
            assert_eq!(h.free(interior), Err(HeapError::NotFromHeap));
            h.free(a).unwrap();
        }
    }

    #[test]
    fn growth_and_exhaustion() {
        let mut h = heap(true, 2); // two 4 KiB chunks total
        let live_block = h.alloc(layout(64)).unwrap();
        let mut live = vec![live_block];
        // Fill until the provider refuses.
        while let Some(p) = h.alloc(layout(512)) {
            live.push(p);
        }
        assert_eq!(h.chunk_count(), 2);
        assert_eq!(h.bytes_reserved(), 8192);
        assert!(live.len() > 4);
        // Everything frees cleanly; accounting returns to just the keeper.
        for p in live.drain(1..) {
            unsafe { h.free(p).unwrap() };
        }
        unsafe { h.free(live[0]).unwrap() };
        assert_eq!(h.in_use_bytes(), 0);
        assert_eq!(h.blocks_live(), 0);
    }

    #[test]
    fn many_small_then_coalesce() {
        let mut h = heap(true, 8);
        let mut live = Vec::new();
        for i in 0..100 {
            let p = h.alloc(layout(24 + (i % 5) * 8)).expect("small alloc");
            unsafe { core::ptr::write_bytes(p.as_ptr(), i as u8, 24) };
            live.push(p);
        }
        // No overlaps: every pointer distinct and patterns intact.
        for (i, p) in live.iter().enumerate() {
            unsafe { assert_eq!(*p.as_ptr(), i as u8) };
        }
        for p in live {
            unsafe { h.free(p).unwrap() };
        }
        // After full coalescing, chunk 1 is one block again.
        let big = h.alloc(layout(3968)).expect("coalesced chunk fits");
        unsafe { h.free(big).unwrap() };
    }

    #[test]
    fn release_chunks_resets_and_regrows() {
        let mut h = heap(true, 4);
        // Two 2000-byte allocations cannot share one 4 KiB chunk
        // (2 x (32 + 2000->2000+8 aligned) > 4064) — forces two chunks.
        let a = h.alloc(layout(2000)).expect("alloc a");
        let b = h.alloc(layout(2000)).expect("alloc b");
        assert_eq!(h.chunk_count(), 2);
        unsafe {
            h.free(a).unwrap();
            h.free(b).unwrap();
        }
        let mut released = Vec::new();
        // SAFETY: heap is empty (both frees above); no pointers survive.
        unsafe { h.release_chunks(|p, bytes| released.push((p.as_ptr() as usize, bytes))) };
        assert_eq!(released.len(), 2, "both chunks must be handed back");
        assert!(released.iter().all(|(_, b)| *b == 4096));
        // Newest-first ordering: the second chunk was pushed to the head.
        assert!(released[0].0 > released[1].0, "chunk list is newest-first");
        assert_eq!(h.chunk_count(), 0);
        assert_eq!(h.bytes_reserved(), 0);
        assert_eq!(h.in_use_bytes(), 0);
        assert_eq!(h.alloc_count(), 2, "lifetime stat survives release");
        // Pristine behavior: the next alloc regrows from the provider.
        let c = h.alloc(layout(100)).expect("regrow alloc");
        assert_eq!(h.chunk_count(), 1);
        unsafe { h.free(c).unwrap() };
        assert_eq!(h.in_use_bytes(), 0);
    }

    #[test]
    #[should_panic(expected = "live allocations")]
    fn release_chunks_refuses_nonempty_heap() {
        let mut h = heap(true, 4);
        // Deliberately leaked for this test: the assert must fire.
        let _live = h.alloc(layout(8)).expect("alloc");
        // SAFETY: contract intentionally violated — that is the test.
        unsafe { h.release_chunks(|_, _| {}) };
    }

    #[test]
    fn stress_lcg_alloc_free_with_reference_model() {
        let mut h = heap(true, 16);
        let mut rng_state = 0x2545_F491_4F6C_DD1Du64;
        let mut rng = move || {
            // xorshift64*
            rng_state ^= rng_state >> 12;
            rng_state ^= rng_state << 25;
            rng_state ^= rng_state >> 27;
            rng_state.wrapping_mul(0x2545_F491_4F6C_DD1D)
        };
        // Model: live ranges keyed by address, checked for disjointness.
        let mut model: BTreeMap<usize, usize> = BTreeMap::new();
        let mut slots: Vec<Option<NonNull<u8>>> = vec![None; 32];
        let mut max_live_bytes = 0usize;

        for round in 0..4000u32 {
            let i = (rng() >> 33) as usize % slots.len();
            match &slots[i] {
                None => {
                    let size = 8 + (rng() >> 29) as usize % 600;
                    if let Some(p) = h.alloc(layout(size)) {
                        let addr = p.as_ptr() as usize;
                        // Disjoint from every live range?
                        for (&a, &s) in &model {
                            assert!(
                                addr + size <= a || a + s <= addr,
                                "round {round}: overlap at {addr:#x} len {size} vs {a:#x} len {s}"
                            );
                        }
                        model.insert(addr, size);
                        max_live_bytes = max_live_bytes.max(h.in_use_bytes());
                        unsafe {
                            (p.as_ptr() as *mut u64).write_unaligned(addr as u64 ^ 0xBEEF);
                        }
                        slots[i] = Some(p);
                    }
                }
                Some(p) => {
                    let addr = p.as_ptr() as usize;
                    // Payload still holds its tag (no cross-writes)?
                    unsafe {
                        assert_eq!(
                            (p.as_ptr() as *const u64).read_unaligned(),
                            addr as u64 ^ 0xBEEF,
                            "round {round}: payload tag corrupted at {addr:#x}"
                        );
                        h.free(*p).unwrap();
                    }
                    assert!(
                        model.remove(&addr).is_some(),
                        "round {round}: model lost live range {addr:#x}"
                    );
                    slots[i] = None;
                }
            }
        }
        // Free everything still live; accounting must return to zero.
        for p in slots.iter().flatten() {
            let addr = p.as_ptr() as usize;
            unsafe { h.free(*p).unwrap() };
            model.remove(&addr);
        }
        assert!(model.is_empty());
        assert_eq!(h.blocks_live(), 0);
        assert_eq!(h.in_use_bytes(), 0);
        assert!(max_live_bytes > 0);
        // Post-stress: full coalescing gives one big block per chunk.
        let big = h.alloc(layout(3968)).expect("post-stress coalescing");
        unsafe { h.free(big).unwrap() };
    }
}
