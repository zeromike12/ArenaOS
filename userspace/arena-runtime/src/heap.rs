//! A bounded, process-local native heap for single-user-thread applications.
//!
//! Each lazy 4 KiB page is an owned frame mapped through the existing
//! `SYS_ALLOC_FRAME`/`SYS_MAP_MEMORY` ABI. Blocks up to 4,080 bytes and 16-byte
//! alignment are supported. The 32-page ceiling is a runtime bound, not a
//! kernel limit; normal process teardown reclaims every mapping and page-table
//! frame. Deallocated blocks are coalesced and reused, but mapped pages are
//! retained until process exit because the native ABI has no ordinary-frame
//! unmap operation. Slot 63 is reserved transiently for the allocator's frame
//! cap; ABI-v2 startup descriptors occupy only slots 1 through 4.
//!
//! The lock serializes allocator state across preemption. The current public
//! process lifecycle creates one user thread per process. If user threads are
//! enabled, their syscalls must also recognize heap mappings in each thread's
//! user-range validation table; sharing only the PML4 is insufficient.

use core::{
    alloc::{GlobalAlloc, Layout},
    cell::UnsafeCell,
    ptr,
    sync::atomic::{AtomicBool, Ordering},
};

use arena_lib::abi::{SYS_ALLOC_FRAME, SYS_CAP_DESTROY, SYS_MAP_MEMORY, syscall1, syscall2};

pub const PAGE_BYTES: usize = 4096;
pub const MAX_HEAP_PAGES: usize = 32;
/// Maximum virtual capacity reserved by the scalable process heap (16 MiB).
/// Physical frames remain lazy and are committed only as allocations need them.
pub const MAX_SCALABLE_HEAP_PAGES: usize = 4096;
pub const MAX_ALLOCATION_BYTES: usize = PAGE_BYTES - HEADER_BYTES;
pub const HEAP_CAP_SLOT: u64 = 63;

const ALIGNMENT: usize = 16;
const HEADER_BYTES: usize = 16;
const MIN_BLOCK_BYTES: usize = 32;
const FREE_FLAG: u32 = 1;
const SIZE_MASK: u32 = !(ALIGNMENT as u32 - 1);

#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct BlockHeader {
    size_flags: u32,
    previous_size: u32,
    reserved: u64,
}

#[derive(Clone, Copy)]
struct HeapState {
    pages: [usize; MAX_HEAP_PAGES],
    page_count: usize,
}

impl HeapState {
    const fn new() -> Self {
        Self {
            pages: [0; MAX_HEAP_PAGES],
            page_count: 0,
        }
    }

    /// # Safety
    /// Every pointer returned by `map_page` must designate a unique, writable,
    /// page-aligned 4 KiB mapping owned by this process. The closure must
    /// leave no mapping/cap behind when returning `None`.
    unsafe fn allocate(
        &mut self,
        layout: Layout,
        mut map_page: impl FnMut() -> Option<usize>,
    ) -> *mut u8 {
        let Some(needed) = block_size(layout) else {
            return ptr::null_mut();
        };

        for &base in &self.pages[..self.page_count] {
            // SAFETY: stored page bases satisfy this method's mapping contract.
            match unsafe { allocate_from_page(base, needed) } {
                Ok(Some(allocation)) => return allocation,
                Ok(None) => {}
                Err(()) => return ptr::null_mut(),
            }
        }

        if self.page_count == MAX_HEAP_PAGES {
            return ptr::null_mut();
        }
        let Some(base) = map_page() else {
            return ptr::null_mut();
        };
        if base == 0
            || !base.is_multiple_of(PAGE_BYTES)
            || self.pages[..self.page_count].contains(&base)
        {
            return ptr::null_mut();
        }

        // SAFETY: the mapping closure returned a unique, owned RW page.
        unsafe { initialize_page(base) };
        self.pages[self.page_count] = base;
        self.page_count += 1;

        // A freshly initialized page always has one free block spanning all
        // 4 KiB, so every accepted layout fits. Retain the page if a corrupted
        // header is ever observed; returning null is fail-closed.
        // SAFETY: this base was just initialized and recorded above.
        match unsafe { allocate_from_page(base, needed) } {
            Ok(Some(allocation)) => allocation,
            Ok(None) | Err(()) => ptr::null_mut(),
        }
    }

    /// # Safety
    /// `allocation` must be null or a pointer previously returned by this
    /// allocator and not yet freed. Unknown, malformed, and double-freed
    /// pointers are ignored without changing allocator state.
    unsafe fn deallocate(&mut self, allocation: *mut u8) {
        if allocation.is_null() {
            return;
        }
        let address = allocation as usize;
        let Some(&base) = self.pages[..self.page_count]
            .iter()
            .find(|&&base| address > base && address < base + PAGE_BYTES)
        else {
            return;
        };
        let payload_offset = address - base;
        if payload_offset < HEADER_BYTES
            || !(payload_offset - HEADER_BYTES).is_multiple_of(ALIGNMENT)
        {
            return;
        }
        let block_offset = payload_offset - HEADER_BYTES;
        // SAFETY: the address is within a known page and aligned to the block
        // header. The helper validates every size/link before the write.
        unsafe { free_in_page(base, block_offset) };
    }
}

/// Thread-safe wrapper around fixed allocator metadata. No kernel resource is
/// requested until the first successful allocation in a process.
pub struct BoundedHeap {
    held: AtomicBool,
    state: UnsafeCell<HeapState>,
}

// SAFETY: all accesses to `state` occur while `held` is acquired; mapped block
// headers are modified under the same lock.
unsafe impl Sync for BoundedHeap {}

impl BoundedHeap {
    pub const fn new() -> Self {
        Self {
            held: AtomicBool::new(false),
            state: UnsafeCell::new(HeapState::new()),
        }
    }

    fn lock(&self) -> HeapGuard<'_> {
        while self
            .held
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        HeapGuard(&self.held)
    }
}

impl Default for BoundedHeap {
    fn default() -> Self {
        Self::new()
    }
}

struct HeapGuard<'a>(&'a AtomicBool);

impl Drop for HeapGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

// SAFETY: `alloc` returns disjoint, suitably aligned blocks or null; `dealloc`
// accepts only a live pointer under the GlobalAlloc contract. Every access to
// the page list and in-page metadata is serialized by `held`.
unsafe impl GlobalAlloc for BoundedHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _guard = self.lock();
        // SAFETY: the kernel mapping routine either returns one owned writable
        // page or rolls its transient capability back before returning None.
        unsafe { (&mut *self.state.get()).allocate(layout, map_heap_page) }
    }

    unsafe fn dealloc(&self, allocation: *mut u8, _layout: Layout) {
        let _guard = self.lock();
        // SAFETY: upheld by GlobalAlloc's caller contract.
        unsafe { (&mut *self.state.get()).deallocate(allocation) };
    }
}

const PAGE_UNCOMMITTED: u8 = 0;
const PAGE_FREE_COMMITTED: u8 = 1;
const PAGE_SMALL: u8 = 2;
const PAGE_LARGE_START: u8 = 3;
const PAGE_LARGE_CONT: u8 = 4;
const LARGE_HEADER_MAGIC: u64 = 0x4152_454E_4131_3348;
const MAX_LARGE_ALIGNMENT: usize = 2 * 1024 * 1024;
const MAX_VM_COMMIT_PAGES: usize = 64;

#[repr(C)]
#[derive(Clone, Copy)]
struct LargeHeader {
    magic: u64,
    payload_offset: u32,
    pages: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HeapUsage {
    pub reserved_pages: u32,
    pub committed_pages: u32,
    pub small_pages: u32,
    pub large_pages: u32,
}

struct ScalableState {
    base: usize,
    capacity: usize,
    page_kind: [u8; MAX_SCALABLE_HEAP_PAGES],
    page_head: [u16; MAX_SCALABLE_HEAP_PAGES],
    page_aux: [u32; MAX_SCALABLE_HEAP_PAGES],
}

impl ScalableState {
    const fn new() -> Self {
        Self {
            base: 0,
            capacity: 0,
            page_kind: [PAGE_UNCOMMITTED; MAX_SCALABLE_HEAP_PAGES],
            page_head: [0; MAX_SCALABLE_HEAP_PAGES],
            page_aux: [0; MAX_SCALABLE_HEAP_PAGES],
        }
    }

    fn configure(&mut self, base: usize, capacity: usize) -> bool {
        if base == 0
            || !base.is_multiple_of(PAGE_BYTES)
            || capacity == 0
            || capacity > MAX_SCALABLE_HEAP_PAGES
            || self.capacity != 0
        {
            return false;
        }
        self.base = base;
        self.capacity = capacity;
        true
    }

    /// # Safety
    /// The base must be configured, and commit must map and zero each page on
    /// success.
    unsafe fn allocate(
        &mut self,
        layout: Layout,
        mut commit: impl FnMut(usize, usize) -> bool,
    ) -> *mut u8 {
        if self.capacity == 0 {
            return ptr::null_mut();
        }
        if layout.align() <= ALIGNMENT
            && let Some(needed) = block_size(layout)
        {
            for index in 0..self.capacity {
                if self.page_kind[index] != PAGE_SMALL {
                    continue;
                }
                let base = self.base + index * PAGE_BYTES;
                // SAFETY: PAGE_SMALL pages were committed and initialized.
                match unsafe { allocate_from_page(base, needed) } {
                    Ok(Some(allocation)) => return allocation,
                    Ok(None) => {}
                    Err(()) => return ptr::null_mut(),
                }
            }
            for index in 0..self.capacity {
                if self.page_kind[index] != PAGE_UNCOMMITTED
                    && self.page_kind[index] != PAGE_FREE_COMMITTED
                {
                    continue;
                }
                if self.page_kind[index] == PAGE_UNCOMMITTED {
                    if !commit(index, 1) {
                        return ptr::null_mut();
                    }
                    self.page_kind[index] = PAGE_FREE_COMMITTED;
                }
                let base = self.base + index * PAGE_BYTES;
                // SAFETY: this page is committed, zeroed, and wholly free.
                unsafe { initialize_page(base) };
                self.page_kind[index] = PAGE_SMALL;
                self.page_head[index] = index as u16 + 1;
                // SAFETY: a fresh page has one free 4 KiB block.
                return match unsafe { allocate_from_page(base, needed) } {
                    Ok(Some(allocation)) => allocation,
                    Ok(None) | Err(()) => ptr::null_mut(),
                };
            }
            return ptr::null_mut();
        }

        let alignment = layout.align();
        if alignment > MAX_LARGE_ALIGNMENT {
            return ptr::null_mut();
        }
        let requested = layout.size().max(1);
        let Some(worst_bytes) = requested
            .checked_add(core::mem::size_of::<LargeHeader>())
            .and_then(|bytes| bytes.checked_add(alignment - 1))
        else {
            return ptr::null_mut();
        };
        let worst_pages = worst_bytes.div_ceil(PAGE_BYTES);
        if worst_pages == 0 || worst_pages > self.capacity {
            return ptr::null_mut();
        }

        for start in 0..=self.capacity - worst_pages {
            if !(start..start + worst_pages).all(|index| {
                self.page_kind[index] == PAGE_UNCOMMITTED
                    || self.page_kind[index] == PAGE_FREE_COMMITTED
            }) {
                continue;
            }
            let allocation_base = self.base + start * PAGE_BYTES;
            let Some(payload_unaligned) =
                allocation_base.checked_add(core::mem::size_of::<LargeHeader>())
            else {
                return ptr::null_mut();
            };
            let Some(payload) = payload_unaligned.checked_add(alignment - 1) else {
                return ptr::null_mut();
            };
            let payload = payload & !(alignment - 1);
            let Some(end) = payload.checked_add(requested) else {
                return ptr::null_mut();
            };
            let pages = (end - allocation_base).div_ceil(PAGE_BYTES);
            if pages == 0 || pages > worst_pages {
                return ptr::null_mut();
            }
            if !self.ensure_committed(start, pages, &mut commit) {
                return ptr::null_mut();
            }
            let payload_offset = payload - allocation_base;
            // SAFETY: first page is committed RW; the header precedes payload.
            unsafe {
                (allocation_base as *mut LargeHeader).write(LargeHeader {
                    magic: LARGE_HEADER_MAGIC,
                    payload_offset: payload_offset as u32,
                    pages: pages as u32,
                });
            }
            for index in start..start + pages {
                self.page_kind[index] = if index == start {
                    PAGE_LARGE_START
                } else {
                    PAGE_LARGE_CONT
                };
                self.page_head[index] = start as u16 + 1;
                self.page_aux[index] = if index == start { pages as u32 } else { 0 };
            }
            return payload as *mut u8;
        }
        ptr::null_mut()
    }

    fn ensure_committed(
        &mut self,
        start: usize,
        pages: usize,
        commit: &mut impl FnMut(usize, usize) -> bool,
    ) -> bool {
        let end = start + pages;
        let mut index = start;
        while index < end {
            if self.page_kind[index] != PAGE_UNCOMMITTED {
                index += 1;
                continue;
            }
            let run_start = index;
            while index < end && self.page_kind[index] == PAGE_UNCOMMITTED {
                index += 1;
            }
            let mut committed = 0;
            while committed < index - run_start {
                let count = (index - run_start - committed).min(MAX_VM_COMMIT_PAGES);
                if !commit(run_start + committed, count) {
                    return false;
                }
                self.page_kind[run_start + committed..run_start + committed + count]
                    .fill(PAGE_FREE_COMMITTED);
                committed += count;
            }
        }
        true
    }

    /// # Safety
    /// allocation must be null or a live pointer returned by allocate.
    unsafe fn deallocate(&mut self, allocation: *mut u8) {
        if allocation.is_null() || self.capacity == 0 {
            return;
        }
        let address = allocation as usize;
        let Some(end) = self.base.checked_add(self.capacity * PAGE_BYTES) else {
            return;
        };
        if address < self.base || address >= end {
            return;
        }
        let page = (address - self.base) / PAGE_BYTES;
        match self.page_kind[page] {
            PAGE_SMALL => {
                let base = self.base + page * PAGE_BYTES;
                if address < base + HEADER_BYTES {
                    return;
                }
                // SAFETY: page is a committed small-allocation page.
                unsafe { free_in_page(base, address - base - HEADER_BYTES) };
            }
            PAGE_LARGE_START | PAGE_LARGE_CONT => {
                let Some(start) = self.page_head[page].checked_sub(1).map(usize::from) else {
                    return;
                };
                if start >= self.capacity || self.page_kind[start] != PAGE_LARGE_START {
                    return;
                }
                let base = self.base + start * PAGE_BYTES;
                // SAFETY: start is an allocator-owned committed page.
                let header = unsafe { (base as *const LargeHeader).read() };
                if header.magic != LARGE_HEADER_MAGIC
                    || address != base + header.payload_offset as usize
                    || header.pages == 0
                    || start + header.pages as usize > self.capacity
                    || self.page_aux[start] != header.pages
                {
                    return;
                }
                for index in start..start + header.pages as usize {
                    let expected = if index == start {
                        PAGE_LARGE_START
                    } else {
                        PAGE_LARGE_CONT
                    };
                    if self.page_head[index] != start as u16 + 1
                        || self.page_kind[index] != expected
                    {
                        return;
                    }
                }
                for index in start..start + header.pages as usize {
                    self.page_kind[index] = PAGE_FREE_COMMITTED;
                    self.page_head[index] = 0;
                    self.page_aux[index] = 0;
                }
            }
            _ => {}
        }
    }

    fn usage(&self, query: crate::vm::Query) -> HeapUsage {
        HeapUsage {
            reserved_pages: query.capacity_pages,
            committed_pages: query.committed_pages,
            small_pages: self.page_kind[..self.capacity]
                .iter()
                .filter(|&&kind| kind == PAGE_SMALL)
                .count() as u32,
            large_pages: self.page_kind[..self.capacity]
                .iter()
                .filter(|&&kind| kind == PAGE_LARGE_START || kind == PAGE_LARGE_CONT)
                .count() as u32,
        }
    }
}

/// Lazy, process-wide allocator backed by one guarded native VM reservation.
/// Small blocks share pages; larger or over-aligned allocations use page runs.
pub struct ScalableHeap {
    held: AtomicBool,
    state: UnsafeCell<ScalableState>,
    region: UnsafeCell<Option<crate::vm::Region>>,
}

// SAFETY: the heap lock protects metadata, VM operations, and block headers
// across all user threads sharing this process allocator.
unsafe impl Sync for ScalableHeap {}

impl ScalableHeap {
    pub const fn new() -> Self {
        Self {
            held: AtomicBool::new(false),
            state: UnsafeCell::new(ScalableState::new()),
            region: UnsafeCell::new(None),
        }
    }

    fn lock(&self) -> HeapGuard<'_> {
        while self
            .held
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        HeapGuard(&self.held)
    }

    /// Return VM-backed accounting without forcing the reservation to exist.
    pub fn query(&self) -> Result<HeapUsage, crate::vm::Error> {
        let _guard = self.lock();
        // SAFETY: protected by held; the region is only created by alloc.
        let region = unsafe { &*self.region.get() };
        let Some(region) = region.as_ref() else {
            return Ok(HeapUsage::default());
        };
        let query = region.query()?;
        // SAFETY: protected by held; query describes this region.
        let state = unsafe { &*self.state.get() };
        Ok(state.usage(query))
    }
}

impl Default for ScalableHeap {
    fn default() -> Self {
        Self::new()
    }
}

unsafe impl GlobalAlloc for ScalableHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _guard = self.lock();
        // SAFETY: the lock serializes the paired region and allocator state.
        let region_slot = unsafe { &mut *self.region.get() };
        if region_slot.is_none() {
            let Ok(region) = crate::vm::Region::reserve(MAX_SCALABLE_HEAP_PAGES as u32) else {
                return ptr::null_mut();
            };
            // SAFETY: the reservation is fresh, aligned, and bounded.
            if !unsafe {
                (&mut *self.state.get()).configure(region.base() as usize, region.pages() as usize)
            } {
                let _ = region.release();
                return ptr::null_mut();
            }
            *region_slot = Some(region);
        }
        let Some(region) = region_slot.as_ref() else {
            return ptr::null_mut();
        };
        // SAFETY: region and allocator state are protected by held.
        unsafe {
            (&mut *self.state.get()).allocate(layout, |start, pages| {
                let mut committed = 0;
                while committed < pages {
                    let count = (pages - committed).min(MAX_VM_COMMIT_PAGES);
                    if region
                        .commit(
                            (start + committed) as u32,
                            count as u32,
                            crate::vm::Protection::READ_WRITE,
                        )
                        .is_err()
                    {
                        return false;
                    }
                    committed += count;
                }
                true
            })
        }
    }

    unsafe fn dealloc(&self, allocation: *mut u8, _layout: Layout) {
        let _guard = self.lock();
        // SAFETY: upheld by GlobalAlloc's caller contract; state is locked.
        unsafe { (&mut *self.state.get()).deallocate(allocation) };
    }
}

fn block_size(layout: Layout) -> Option<usize> {
    if layout.align() > ALIGNMENT {
        return None;
    }
    let requested = layout.size().max(1);
    let with_header = requested.checked_add(HEADER_BYTES)?;
    let rounded = with_header.checked_add(ALIGNMENT - 1)? & !(ALIGNMENT - 1);
    (rounded <= PAGE_BYTES).then_some(rounded)
}

/// Allocate one owned frame and consume it into an NX writable user mapping.
/// Mapping refusal discards the still-owned Untyped cap so OOM is mutation-free.
fn map_heap_page() -> Option<usize> {
    // SAFETY: slot 63 is reserved by this runtime and is empty at process
    // startup; SYS_ALLOC_FRAME refuses without replacing an occupied slot.
    if unsafe { syscall1(SYS_ALLOC_FRAME, HEAP_CAP_SLOT) } <= 0 {
        return None;
    }
    // SAFETY: the slot now holds the owned frame just issued above.
    let mapped = unsafe { syscall2(SYS_MAP_MEMORY, HEAP_CAP_SLOT, 1) };
    if mapped <= 0 {
        // SAFETY: on refusal SYS_MAP_MEMORY leaves the owned cap in slot 63.
        let _ = unsafe { syscall1(SYS_CAP_DESTROY, HEAP_CAP_SLOT) };
        return None;
    }
    let base = mapped as usize;
    base.is_multiple_of(PAGE_BYTES).then_some(base)
}

unsafe fn initialize_page(base: usize) {
    // SAFETY: the mapped page is writable, owned by this process, and has not
    // been exposed to an allocation yet. Clearing it prevents stale-frame
    // contents from crossing process lifetimes.
    unsafe { ptr::write_bytes(base as *mut u8, 0, PAGE_BYTES) };
    // SAFETY: base is 4 KiB aligned and the page was just zeroed.
    unsafe {
        (base as *mut BlockHeader).write(BlockHeader {
            size_flags: PAGE_BYTES as u32 | FREE_FLAG,
            previous_size: 0,
            reserved: 0,
        });
    }
}

fn read_header(base: usize, offset: usize) -> Result<BlockHeader, ()> {
    if !offset.is_multiple_of(ALIGNMENT) || offset + HEADER_BYTES > PAGE_BYTES {
        return Err(());
    }
    // SAFETY: callers validate page ownership and this range check proves the
    // aligned header fits inside that page.
    let header = unsafe { ((base + offset) as *const BlockHeader).read() };
    let size = (header.size_flags & SIZE_MASK) as usize;
    if header.size_flags & !(SIZE_MASK | FREE_FLAG) != 0
        || size < MIN_BLOCK_BYTES
        || !size.is_multiple_of(ALIGNMENT)
        || offset + size > PAGE_BYTES
        || (header.previous_size != 0 && (header.previous_size as usize) < MIN_BLOCK_BYTES)
        || !((header.previous_size as usize).is_multiple_of(ALIGNMENT))
        || header.previous_size as usize > offset
    {
        return Err(());
    }
    Ok(header)
}

unsafe fn write_header(base: usize, offset: usize, header: BlockHeader) {
    // SAFETY: internal callers have validated page bounds and alignment.
    unsafe { ((base + offset) as *mut BlockHeader).write(header) };
}

unsafe fn allocate_from_page(base: usize, needed: usize) -> Result<Option<*mut u8>, ()> {
    let mut offset = 0usize;
    while offset < PAGE_BYTES {
        let header = read_header(base, offset)?;
        let block_size = (header.size_flags & SIZE_MASK) as usize;
        if header.size_flags & FREE_FLAG != 0 && block_size >= needed {
            let remainder = block_size - needed;
            let allocated_size = if remainder >= MIN_BLOCK_BYTES {
                needed
            } else {
                block_size
            };
            // SAFETY: this header was just read and validated.
            unsafe {
                write_header(
                    base,
                    offset,
                    BlockHeader {
                        size_flags: allocated_size as u32,
                        previous_size: header.previous_size,
                        reserved: 0,
                    },
                );
            }
            if allocated_size < block_size {
                let remainder_offset = offset + allocated_size;
                let remainder_size = block_size - allocated_size;
                // SAFETY: the remaining span is aligned, at least 32 bytes,
                // and lies inside the validated original free block.
                unsafe {
                    write_header(
                        base,
                        remainder_offset,
                        BlockHeader {
                            size_flags: remainder_size as u32 | FREE_FLAG,
                            previous_size: allocated_size as u32,
                            reserved: 0,
                        },
                    );
                }
                let after = remainder_offset + remainder_size;
                if after < PAGE_BYTES {
                    let mut next = read_header(base, after)?;
                    next.previous_size = remainder_size as u32;
                    // SAFETY: the next header was validated above.
                    unsafe { write_header(base, after, next) };
                }
            } else {
                let after = offset + allocated_size;
                if after < PAGE_BYTES {
                    let mut next = read_header(base, after)?;
                    next.previous_size = allocated_size as u32;
                    // SAFETY: the next header was validated above.
                    unsafe { write_header(base, after, next) };
                }
            }
            return Ok(Some((base + offset + HEADER_BYTES) as *mut u8));
        }
        offset += block_size;
    }
    Ok(None)
}

unsafe fn free_in_page(base: usize, offset: usize) {
    let Ok(mut header) = read_header(base, offset) else {
        return;
    };
    if header.size_flags & FREE_FLAG != 0 {
        return;
    }
    let mut start = offset;
    let mut total = (header.size_flags & SIZE_MASK) as usize;
    header.size_flags |= FREE_FLAG;
    // SAFETY: the header is valid and the allocation's previous/next blocks
    // are changed only after their own validation.
    unsafe { write_header(base, offset, header) };

    if header.previous_size != 0 {
        let previous_offset = offset - header.previous_size as usize;
        if let Ok(previous) = read_header(base, previous_offset) {
            let previous_block = (previous.size_flags & SIZE_MASK) as usize;
            if previous_block == header.previous_size as usize
                && previous.size_flags & FREE_FLAG != 0
            {
                start = previous_offset;
                total += previous_block;
                header = BlockHeader {
                    size_flags: total as u32 | FREE_FLAG,
                    previous_size: previous.previous_size,
                    reserved: 0,
                };
                // SAFETY: adjacent validated blocks combine into a span
                // contained by this page.
                unsafe { write_header(base, start, header) };
            }
        }
    }

    let next_offset = start + total;
    if next_offset < PAGE_BYTES
        && let Ok(next) = read_header(base, next_offset)
        && next.size_flags & FREE_FLAG != 0
    {
        total += (next.size_flags & SIZE_MASK) as usize;
        header.size_flags = total as u32 | FREE_FLAG;
        // SAFETY: adjacent validated blocks combine into one span.
        unsafe { write_header(base, start, header) };
    }
    let after = start + total;
    if after < PAGE_BYTES
        && let Ok(mut next) = read_header(base, after)
    {
        next.previous_size = total as u32;
        // SAFETY: the following block header was validated above.
        unsafe { write_header(base, after, next) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[repr(C, align(4096))]
    #[derive(Clone, Copy)]
    struct TestPage([u8; PAGE_BYTES]);

    impl TestPage {
        const fn filled(value: u8) -> Self {
            Self([value; PAGE_BYTES])
        }
    }

    #[test]
    fn blocks_align_reuse_and_coalesce_without_touching_neighbor_pages() {
        let mut pages = [TestPage::filled(0xA5); 2];
        let mut next_page = 0usize;
        let mut state = HeapState::new();
        let small = Layout::from_size_align(32, 16).unwrap();
        let medium = Layout::from_size_align(64, 8).unwrap();
        let first = unsafe {
            state.allocate(small, || {
                let page = pages.get_mut(next_page)?;
                next_page += 1;
                Some(page.0.as_mut_ptr() as usize)
            })
        };
        let second = unsafe {
            state.allocate(medium, || {
                let page = pages.get_mut(next_page)?;
                next_page += 1;
                Some(page.0.as_mut_ptr() as usize)
            })
        };
        assert!(!first.is_null() && !second.is_null());
        assert_eq!((first as usize) & (ALIGNMENT - 1), 0);
        assert_eq!((second as usize) & (ALIGNMENT - 1), 0);
        assert_eq!(next_page, 1, "the allocator reuses a partially free page");
        // The mapped page is scrubbed before its block headers are installed.
        assert!(
            unsafe { core::slice::from_raw_parts(first, 32) }
                .iter()
                .all(|byte| *byte == 0)
        );
        assert!(
            unsafe { core::slice::from_raw_parts(second, 64) }
                .iter()
                .all(|byte| *byte == 0)
        );

        unsafe { state.deallocate(first) };
        let reused = unsafe { state.allocate(small, || None) };
        assert_eq!(reused, first, "a freed block is reused in place");
        unsafe {
            state.deallocate(second);
            state.deallocate(reused);
        }
        let whole_page = Layout::from_size_align(MAX_ALLOCATION_BYTES, 16).unwrap();
        let large = unsafe { state.allocate(whole_page, || None) };
        assert_eq!(large, pages[0].0.as_mut_ptr().wrapping_add(HEADER_BYTES));
        assert_eq!(next_page, 1, "coalescing returned the whole page span");
    }

    #[test]
    fn capacity_refusal_is_mutation_free_and_a_free_large_block_recovers() {
        let mut pages = [TestPage::filled(0xCC); MAX_HEAP_PAGES];
        let mut next_page = 0usize;
        let mut state = HeapState::new();
        let full_page = Layout::from_size_align(MAX_ALLOCATION_BYTES, 16).unwrap();
        let mut allocations = [ptr::null_mut(); MAX_HEAP_PAGES];
        for allocation in &mut allocations {
            *allocation = unsafe {
                state.allocate(full_page, || {
                    let page = pages.get_mut(next_page)?;
                    next_page += 1;
                    Some(page.0.as_mut_ptr() as usize)
                })
            };
            assert!(!allocation.is_null());
        }
        assert_eq!(state.page_count, MAX_HEAP_PAGES);
        let refusal = unsafe {
            state.allocate(full_page, || {
                panic!("a full allocator must refuse before asking for another frame")
            })
        };
        assert!(refusal.is_null());
        assert_eq!(state.page_count, MAX_HEAP_PAGES);
        assert_eq!(next_page, MAX_HEAP_PAGES);

        let expected_recovered = allocations[11];
        unsafe { state.deallocate(expected_recovered) };
        let recovered = unsafe { state.allocate(full_page, || None) };
        assert_eq!(recovered, expected_recovered);
        for allocation in allocations {
            if allocation != expected_recovered {
                unsafe { state.deallocate(allocation) };
            }
        }
        unsafe { state.deallocate(recovered) };
    }

    #[test]
    fn unsupported_layouts_and_frame_oom_leave_state_unchanged() {
        let mut state = HeapState::new();
        let too_aligned = Layout::from_size_align(8, 32).unwrap();
        let too_large = Layout::from_size_align(PAGE_BYTES, 16).unwrap();
        let mut provider_called = false;
        assert!(
            unsafe {
                state.allocate(too_aligned, || {
                    provider_called = true;
                    None
                })
            }
            .is_null()
        );
        assert!(unsafe { state.allocate(too_large, || None) }.is_null());
        assert!(
            unsafe { state.allocate(Layout::from_size_align(8, 8).unwrap(), || None) }.is_null()
        );
        assert!(!provider_called);
        assert_eq!(state.page_count, 0);
    }

    #[test]
    fn scalable_heap_commits_lazily_aligns_reuses_and_refuses_oom() {
        const TEST_PAGES: usize = 80;
        let mut pages = [TestPage::filled(0xA5); TEST_PAGES];
        let base = pages[0].0.as_mut_ptr() as usize;
        let mut state = ScalableState::new();
        assert!(state.configure(base, TEST_PAGES));
        let mut committed = [false; TEST_PAGES];
        let mut commit_calls = 0;

        let large_layout = Layout::from_size_align(256 * 1024, 16).unwrap();
        let large = unsafe {
            state.allocate(large_layout, |start, count| {
                assert!(count <= MAX_VM_COMMIT_PAGES);
                assert!((start..start + count).all(|page| !committed[page]));
                committed[start..start + count].fill(true);
                ptr::write_bytes(
                    (base + start * PAGE_BYTES) as *mut u8,
                    0,
                    count * PAGE_BYTES,
                );
                commit_calls += 1;
                true
            })
        };
        assert!(!large.is_null());
        assert_eq!((large as usize) & 15, 0);
        assert_eq!(commit_calls, 2, "large commits are split at 64 pages");
        assert_eq!(committed.iter().filter(|&&value| value).count(), 65);
        unsafe {
            large.write(0x31);
            large.add(256 * 1024 - 1).write(0x79);
            assert_eq!(large.read(), 0x31);
            assert_eq!(large.add(256 * 1024 - 1).read(), 0x79);
            state.deallocate(large);
        }
        assert!(
            state.page_kind[..65]
                .iter()
                .all(|&kind| kind == PAGE_FREE_COMMITTED)
        );
        let reused = unsafe {
            state.allocate(large_layout, |_, _| {
                panic!("committed pages must be reused")
            })
        };
        assert_eq!(reused, large);
        unsafe { state.deallocate(reused) };

        let aligned_layout = Layout::from_size_align(24, 64 * 1024).unwrap();
        let aligned = unsafe {
            state.allocate(aligned_layout, |start, count| {
                committed[start..start + count].fill(true);
                ptr::write_bytes(
                    (base + start * PAGE_BYTES) as *mut u8,
                    0,
                    count * PAGE_BYTES,
                );
                true
            })
        };
        assert!(!aligned.is_null());
        assert_eq!((aligned as usize) & (64 * 1024 - 1), 0);
        unsafe { state.deallocate(aligned) };

        let before_kinds = state.page_kind;
        let too_large =
            Layout::from_size_align(MAX_SCALABLE_HEAP_PAGES * PAGE_BYTES + 1, 16).unwrap();
        let refusal = unsafe {
            state.allocate(too_large, |_, _| {
                panic!("oversize OOM must not commit pages")
            })
        };
        assert!(refusal.is_null());
        assert_eq!(state.page_kind, before_kinds);
    }
}
