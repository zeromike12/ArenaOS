//! Native byte streams over one bounded SharedRegion page.
//!
//! The caller separately proves authority by holding and validating the exact
//! SharedRegion capability in Startup ABI v2. These types interpret bytes in
//! that mapping; channel numbers and metadata do not grant authority.

use core::{
    cell::Cell,
    cell::UnsafeCell,
    marker::PhantomData,
    mem::{align_of, size_of},
    ptr::{self, NonNull},
    sync::atomic::{AtomicBool, AtomicU32, Ordering},
};

pub const STREAM_PAGE_BYTES: usize = 4096;
pub const STREAM_CHANNELS: usize = 3;
pub const STREAM_CAPACITY: usize = 768;
pub const STREAM_WAKE_BADGE: u64 = 1 << 5;
const MAGIC: u32 = u32::from_le_bytes(*b"ASTR");
const VERSION: u32 = 1;
const WRITER_CLOSED: u32 = 1;
const READER_CLOSED: u32 = 2;
static STANDARD_STREAMS_ATTACHED: AtomicBool = AtomicBool::new(false);

/// The three conventional channels share one independently held stream page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Channel {
    Stdin = 0,
    Stdout = 1,
    Stderr = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    BadAddress,
    TooSmall,
    BadFormat,
    WouldBlock,
    BrokenPipe,
    Closed,
    Corrupt,
    EndpointInUse,
    MissingAuthority,
    CapabilityMismatch,
    RegionGeometry(i64),
    Kernel(i64),
}

#[repr(C, align(64))]
struct Ring {
    write: AtomicU32,
    read: AtomicU32,
    state: AtomicU32,
    reserved: [u8; 52],
    bytes: UnsafeCell<[u8; STREAM_CAPACITY]>,
}

// Safety: each ring has one producer and one consumer. The producer publishes
// bytes before its release-store to write; the consumer acquire-loads that
// counter before reading. Reuse follows the same rule through read. Endpoint
// wrappers are !Send/!Sync, so safe Rust cannot silently add a second peer.
unsafe impl Sync for Ring {}

impl Ring {
    const fn new() -> Self {
        Self {
            write: AtomicU32::new(0),
            read: AtomicU32::new(0),
            state: AtomicU32::new(0),
            reserved: [0; 52],
            bytes: UnsafeCell::new([0; STREAM_CAPACITY]),
        }
    }
}

#[repr(C, align(64))]
struct Region {
    magic: u32,
    version: u32,
    channels: u32,
    capacity: u32,
    reserved: [u8; 48],
    rings: [Ring; STREAM_CHANNELS],
}

const REGION_BYTES: usize = size_of::<Region>();
const _: () = assert!(REGION_BYTES <= STREAM_PAGE_BYTES);
const _: () = assert!(size_of::<Ring>().is_multiple_of(64));

/// A validated view of the ring layout in the caller's exact SharedRegion
/// mapping. Keep the mapping live while any endpoint exists.
pub struct StreamSet {
    region: NonNull<Region>,
    claimed: Cell<u8>,
    not_send_or_sync: PhantomData<*mut ()>,
}

impl StreamSet {
    /// Initialize a newly created, zeroed stream page before publishing it.
    ///
    /// # Safety
    /// memory must be page-aligned, writable for bytes bytes, and have no
    /// other concurrent users.
    pub unsafe fn initialize(memory: *mut u8, bytes: usize) -> Result<Self, Error> {
        let region = region_ptr(memory, bytes)?;
        // SAFETY: guaranteed by the exclusive-new-region contract above.
        unsafe { region.as_ptr().write(Region::new()) };
        Ok(Self {
            region,
            claimed: Cell::new(0),
            not_send_or_sync: PhantomData,
        })
    }

    /// Attach to a stream page initialized by its trusted owner.
    ///
    /// # Safety
    /// memory must map the exact held stream SharedRegion and remain mapped
    /// until this view and all endpoints are no longer used.
    pub unsafe fn attach(memory: *mut u8, bytes: usize) -> Result<Self, Error> {
        let region = region_ptr(memory, bytes)?;
        // SAFETY: the caller promises a readable live mapping.
        let header = unsafe { region.as_ref() };
        if header.magic != MAGIC
            || header.version != VERSION
            || header.channels as usize != STREAM_CHANNELS
            || header.capacity as usize != STREAM_CAPACITY
            || header.reserved.iter().any(|byte| *byte != 0)
            || header
                .rings
                .iter()
                .any(|ring| ring.reserved.iter().any(|byte| *byte != 0))
        {
            return Err(Error::BadFormat);
        }
        Ok(Self {
            region,
            claimed: Cell::new(0),
            not_send_or_sync: PhantomData,
        })
    }

    /// Get the one local producer endpoint for a channel.
    pub fn writer(&self, channel: Channel) -> Result<Writer<'_>, Error> {
        let claim = 1 << channel as u8;
        self.claim(claim)?;
        Ok(Writer {
            ring: self.ring(channel),
            owner: self,
            claim,
            not_send_or_sync: PhantomData,
        })
    }

    /// Get the one local consumer endpoint for a channel.
    pub fn reader(&self, channel: Channel) -> Result<Reader<'_>, Error> {
        let claim = 1 << (channel as u8 + 3);
        self.claim(claim)?;
        Ok(Reader {
            ring: self.ring(channel),
            owner: self,
            claim,
            not_send_or_sync: PhantomData,
        })
    }

    fn claim(&self, bit: u8) -> Result<(), Error> {
        if self.claimed.get() & bit != 0 {
            return Err(Error::EndpointInUse);
        }
        self.claimed.set(self.claimed.get() | bit);
        Ok(())
    }

    fn release(&self, bit: u8) {
        self.claimed.set(self.claimed.get() & !bit);
    }

    fn ring(&self, channel: Channel) -> NonNull<Ring> {
        // SAFETY: this view has a validated live Region and Channel only has
        // the three canonical in-range discriminants.
        let region = unsafe { self.region.as_ref() };
        let pointer = &region.rings[channel as usize] as *const Ring as *mut Ring;
        // SAFETY: the selected array element is non-null and aligned.
        unsafe { NonNull::new_unchecked(pointer) }
    }
}

impl Region {
    const fn new() -> Self {
        Self {
            magic: MAGIC,
            version: VERSION,
            channels: STREAM_CHANNELS as u32,
            capacity: STREAM_CAPACITY as u32,
            reserved: [0; 48],
            rings: [const { Ring::new() }; STREAM_CHANNELS],
        }
    }
}

/// The unique producer endpoint for one SPSC channel.
pub struct Writer<'a> {
    ring: NonNull<Ring>,
    owner: &'a StreamSet,
    claim: u8,
    not_send_or_sync: PhantomData<*mut ()>,
}

impl Writer<'_> {
    /// Write as many bytes as currently fit. A short count is valid; a full
    /// ring returns WouldBlock. The caller sends the peer's wake hint after a
    /// successful write.
    pub fn write(&mut self, input: &[u8]) -> Result<usize, Error> {
        if input.is_empty() {
            return Ok(0);
        }
        // SAFETY: endpoint construction pins the SharedRegion mapping.
        let ring = unsafe { self.ring.as_ref() };
        let state = ring.state.load(Ordering::Acquire);
        if state & READER_CLOSED != 0 {
            return Err(Error::BrokenPipe);
        }
        if state & WRITER_CLOSED != 0 {
            return Err(Error::Closed);
        }
        let head = ring.write.load(Ordering::Relaxed);
        let tail = ring.read.load(Ordering::Acquire);
        let occupied = head.wrapping_sub(tail);
        if occupied > STREAM_CAPACITY as u32 {
            return Err(Error::Corrupt);
        }
        let available = STREAM_CAPACITY - occupied as usize;
        if available == 0 {
            return Err(Error::WouldBlock);
        }
        let count = input.len().min(available);
        let start = head as usize % STREAM_CAPACITY;
        let first = count.min(STREAM_CAPACITY - start);
        // SAFETY: SPSC ownership and the acquired read counter prove the
        // selected free range is not being read by the consumer.
        unsafe {
            let data = (*ring.bytes.get()).as_mut_ptr();
            ptr::copy_nonoverlapping(input.as_ptr(), data.add(start), first);
            if count > first {
                ptr::copy_nonoverlapping(input.as_ptr().add(first), data, count - first);
            }
        }
        ring.write
            .store(head.wrapping_add(count as u32), Ordering::Release);
        Ok(count)
    }

    /// Close the producer side. The consumer drains queued bytes, then reads
    /// EOF as a zero count.
    pub fn close(&mut self) {
        // SAFETY: endpoint construction pins the SharedRegion mapping.
        unsafe { self.ring.as_ref() }
            .state
            .fetch_or(WRITER_CLOSED, Ordering::AcqRel);
    }
}
impl Drop for Writer<'_> {
    fn drop(&mut self) {
        self.owner.release(self.claim);
    }
}

/// The unique consumer endpoint for one SPSC channel.
pub struct Reader<'a> {
    ring: NonNull<Ring>,
    owner: &'a StreamSet,
    claim: u8,
    not_send_or_sync: PhantomData<*mut ()>,
}

impl Reader<'_> {
    /// Read up to the available bytes. A zero count means EOF only after
    /// writer_closed returns true; an empty open ring returns WouldBlock. The
    /// caller wakes the producer after consuming bytes.
    pub fn read(&mut self, output: &mut [u8]) -> Result<usize, Error> {
        if output.is_empty() {
            return Ok(0);
        }
        // SAFETY: endpoint construction pins the SharedRegion mapping.
        let ring = unsafe { self.ring.as_ref() };
        let head = ring.write.load(Ordering::Acquire);
        let tail = ring.read.load(Ordering::Relaxed);
        let available = head.wrapping_sub(tail);
        if available > STREAM_CAPACITY as u32 {
            return Err(Error::Corrupt);
        }
        if available == 0 {
            let state = ring.state.load(Ordering::Acquire);
            if state & WRITER_CLOSED != 0 {
                return Ok(0);
            }
            if state & READER_CLOSED != 0 {
                return Err(Error::Closed);
            }
            return Err(Error::WouldBlock);
        }
        if ring.state.load(Ordering::Relaxed) & READER_CLOSED != 0 {
            return Err(Error::Closed);
        }
        let count = output.len().min(available as usize);
        let start = tail as usize % STREAM_CAPACITY;
        let first = count.min(STREAM_CAPACITY - start);
        // SAFETY: the acquired write counter proves initialization; SPSC
        // gives this consumer sole read ownership until it publishes tail.
        unsafe {
            let data = (*ring.bytes.get()).as_ptr();
            ptr::copy_nonoverlapping(data.add(start), output.as_mut_ptr(), first);
            if count > first {
                ptr::copy_nonoverlapping(data, output.as_mut_ptr().add(first), count - first);
            }
        }
        ring.read
            .store(tail.wrapping_add(count as u32), Ordering::Release);
        Ok(count)
    }

    /// Close the consumer side. Future producer writes fail with BrokenPipe.
    pub fn close(&mut self) {
        // SAFETY: endpoint construction pins the SharedRegion mapping.
        unsafe { self.ring.as_ref() }
            .state
            .fetch_or(READER_CLOSED, Ordering::AcqRel);
    }

    pub fn writer_closed(&self) -> bool {
        // SAFETY: endpoint construction pins the SharedRegion mapping.
        unsafe { self.ring.as_ref() }.state.load(Ordering::Acquire) & WRITER_CLOSED != 0
    }
}
impl Drop for Reader<'_> {
    fn drop(&mut self) {
        self.owner.release(self.claim);
    }
}

/// Startup-validated standard stream mapping. Dropping it removes only the
/// local map; the exact inherited SharedRegion capability remains in the
/// process capability table until ordinary process teardown.
pub struct NativeStreams {
    set: StreamSet,
    map_va: u64,
    stream_cap_slot: u64,
    wake_cap_slot: u64,
    _claim: StandardAttachClaim,
}

/// Parent-side endpoint for one explicitly delegated helper stream set. The
/// helper uses the normal stdin-reader/stdout-writer convention; the owning
/// application writes stdin and reads the helper's output channels.
pub struct HelperStreams {
    set: StreamSet,
    map_va: u64,
    stream_cap_slot: u64,
}

impl HelperStreams {
    /// Adopt the exact SharedRegion returned by the authenticated Desktop
    /// helper service. The slot is consumed even when validation fails.
    pub fn from_cap(stream_cap_slot: u64) -> Result<Self, Error> {
        let owned_slot = stream_cap_slot;
        let adopted = (|| {
            use arena_startup_abi::startup::{
                CAP_KIND_SHARED_REGION, RIGHT_COPY, RIGHT_DESTROY, RIGHT_READ, RIGHT_WRITE,
            };

            let stream = describe_capability(stream_cap_slot).map_err(Error::Kernel)?;
            if stream[0] != u64::from(CAP_KIND_SHARED_REGION)
                || stream[2] != u64::from(RIGHT_READ | RIGHT_WRITE | RIGHT_COPY | RIGHT_DESTROY)
            {
                return Err(Error::CapabilityMismatch);
            }
            // SAFETY: all syscall output buffers are live caller-owned arrays.
            let pages = unsafe {
                arena_lib::abi::syscall6(
                    arena_lib::abi::SYS_SHARED_PAGES,
                    owned_slot,
                    0,
                    0,
                    0,
                    0,
                    0,
                )
            };
            if pages != 1 {
                return Err(Error::RegionGeometry(pages));
            }
            // SAFETY: the exact validated SharedRegion cap grants RW mapping.
            let map =
                unsafe { arena_lib::abi::syscall2(arena_lib::abi::SYS_SHARED_MAP, owned_slot, 1) };
            if map <= 0 {
                return Err(Error::Kernel(map));
            }
            // SAFETY: `map` belongs to this exact validated one-page cap.
            let set = match unsafe { StreamSet::attach(map as *mut u8, STREAM_PAGE_BYTES) } {
                Ok(set) => set,
                Err(error) => {
                    // SAFETY: release only the mapping returned above.
                    let _ = unsafe {
                        arena_lib::abi::syscall6(
                            arena_lib::abi::SYS_SHARED_UNMAP,
                            map as u64,
                            0,
                            0,
                            0,
                            0,
                            0,
                        )
                    };
                    return Err(error);
                }
            };
            Ok(Self {
                set,
                map_va: map as u64,
                stream_cap_slot: owned_slot,
            })
        })();
        if adopted.is_err() {
            destroy_capability(owned_slot);
        }
        adopted
    }

    pub fn set(&self) -> &StreamSet {
        &self.set
    }

    pub fn stream_cap_slot(&self) -> u64 {
        self.stream_cap_slot
    }
}

impl Drop for HelperStreams {
    fn drop(&mut self) {
        if let Ok(mut stdin) = self.set.writer(Channel::Stdin) {
            stdin.close();
        }
        for channel in [Channel::Stdout, Channel::Stderr] {
            // The parent owns only the output consumer side. Closing it tells
            // the helper producer that its peer has gone away; it must not
            // mark the helper's writer as closed on the producer's behalf.
            if let Ok(mut reader) = self.set.reader(channel) {
                reader.close();
            }
        }
        // SAFETY: this wrapper owns the only parent-side mapping and cap slots.
        let _ = unsafe {
            arena_lib::abi::syscall6(arena_lib::abi::SYS_SHARED_UNMAP, self.map_va, 0, 0, 0, 0, 0)
        };
        destroy_capability(self.stream_cap_slot);
    }
}

impl NativeStreams {
    pub fn from_startup(view: &arena_startup_abi::startup::StartupView<'_>) -> Result<Self, Error> {
        use arena_startup_abi::startup as abi;

        let claim = StandardAttachClaim::acquire()?;
        if view.flags() & arena_startup_abi::manifest::FLAG_STANDARD_STREAMS == 0 {
            return Err(Error::MissingAuthority);
        }
        let set_index = view
            .stream_set_descriptor()
            .ok_or(Error::MissingAuthority)?;
        let wake_index = view
            .stream_wake_descriptor()
            .ok_or(Error::MissingAuthority)?;
        if view.stdin_descriptor() != Some(set_index)
            || view.stdout_descriptor() != Some(set_index)
            || view.stderr_descriptor() != Some(set_index)
        {
            return Err(Error::BadFormat);
        }
        let set_cap = view
            .capability(set_index as usize)
            .ok_or(Error::MissingAuthority)?;
        let wake_cap = view
            .capability(wake_index as usize)
            .ok_or(Error::MissingAuthority)?;
        if set_cap.role != abi::CapabilityRole::StandardStreamSet
            || set_cap.kind != abi::CAP_KIND_SHARED_REGION
            || set_cap.rights != (abi::RIGHT_READ | abi::RIGHT_WRITE)
            || wake_cap.role != abi::CapabilityRole::StreamWake
            || wake_cap.kind != abi::CAP_KIND_NOTIFICATION
            || wake_cap.rights != abi::RIGHT_WRITE
        {
            return Err(Error::BadFormat);
        }
        let observed_set = describe_capability(set_cap.slot as u64).map_err(Error::Kernel)?;
        if observed_set[0] != u64::from(set_cap.kind)
            || observed_set[2] != u64::from(set_cap.rights)
        {
            return Err(Error::CapabilityMismatch);
        }
        let observed_wake = describe_capability(wake_cap.slot as u64).map_err(Error::Kernel)?;
        if observed_wake[0] != u64::from(wake_cap.kind)
            || observed_wake[2] != u64::from(wake_cap.rights)
        {
            return Err(Error::CapabilityMismatch);
        }
        // SAFETY: describe_capability proved the cap slots occupied; the
        // exact kind and rights were checked against the startup record.
        let pages = unsafe {
            arena_lib::abi::syscall6(
                arena_lib::abi::SYS_SHARED_PAGES,
                set_cap.slot as u64,
                0,
                0,
                0,
                0,
                0,
            )
        };
        if pages != 1 {
            return Err(Error::RegionGeometry(pages));
        }
        // SAFETY: the exact inherited SharedRegion cap grants RW mapping.
        let map = unsafe {
            arena_lib::abi::syscall2(arena_lib::abi::SYS_SHARED_MAP, set_cap.slot as u64, 1)
        };
        if map <= 0 {
            return Err(Error::Kernel(map));
        }
        // SAFETY: the mapping was returned by SYS_SHARED_MAP for the held
        // SharedRegion cap and covers one page.
        let set = match unsafe { StreamSet::attach(map as *mut u8, STREAM_PAGE_BYTES) } {
            Ok(set) => set,
            Err(error) => {
                // SAFETY: remove only the exact mapping returned above.
                let _ = unsafe {
                    arena_lib::abi::syscall6(
                        arena_lib::abi::SYS_SHARED_UNMAP,
                        map as u64,
                        0,
                        0,
                        0,
                        0,
                        0,
                    )
                };
                return Err(error);
            }
        };
        Ok(Self {
            set,
            map_va: map as u64,
            stream_cap_slot: set_cap.slot as u64,
            wake_cap_slot: wake_cap.slot as u64,
            _claim: claim,
        })
    }

    pub fn set(&self) -> &StreamSet {
        &self.set
    }

    pub fn stream_cap_slot(&self) -> u64 {
        self.stream_cap_slot
    }

    /// Wake the trusted Desktop event loop to recheck stream state. The held
    /// WRITE-only Notification cap is the authority; the badge is a hint.
    pub fn wake_broker(&self) -> Result<(), i64> {
        // SAFETY: this exact slot was validated as a WRITE-only Notification.
        let status = unsafe {
            arena_lib::abi::syscall2(
                arena_lib::abi::SYS_NOTIFY,
                self.wake_cap_slot,
                STREAM_WAKE_BADGE,
            )
        };
        if status == 0 { Ok(()) } else { Err(status) }
    }
}

struct StandardAttachClaim;

impl StandardAttachClaim {
    fn acquire() -> Result<Self, Error> {
        STANDARD_STREAMS_ATTACHED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| Self)
            .map_err(|_| Error::EndpointInUse)
    }
}

impl Drop for StandardAttachClaim {
    fn drop(&mut self) {
        STANDARD_STREAMS_ATTACHED.store(false, Ordering::Release);
    }
}

fn describe_capability(slot: u64) -> Result<[u64; 3], i64> {
    let mut words = [0u64; 3];
    // SAFETY: words is a caller-owned buffer with exact syscall ABI size.
    let status = unsafe {
        arena_lib::abi::syscall2(
            arena_lib::abi::SYS_CAP_DESCRIBE,
            slot,
            words.as_mut_ptr() as u64,
        )
    };
    if status == 0 { Ok(words) } else { Err(status) }
}

fn destroy_capability(slot: u64) {
    // SAFETY: this function is called only for cap slots adopted by runtime.
    let _ = unsafe { arena_lib::abi::syscall1(arena_lib::abi::SYS_CAP_DESTROY, slot) };
}

impl Drop for NativeStreams {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns exactly the mapping returned by
        // SYS_SHARED_MAP in from_startup.
        let _ = unsafe {
            arena_lib::abi::syscall6(arena_lib::abi::SYS_SHARED_UNMAP, self.map_va, 0, 0, 0, 0, 0)
        };
    }
}

fn region_ptr(memory: *mut u8, bytes: usize) -> Result<NonNull<Region>, Error> {
    let Some(memory) = NonNull::new(memory) else {
        return Err(Error::BadAddress);
    };
    if (memory.as_ptr() as usize) & (align_of::<Region>() - 1) != 0 {
        return Err(Error::BadAddress);
    }
    if !(REGION_BYTES..=STREAM_PAGE_BYTES).contains(&bytes) {
        return Err(Error::TooSmall);
    }
    Ok(memory.cast())
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;
    use std::boxed::Box;

    #[repr(align(4096))]
    struct Page([u8; STREAM_PAGE_BYTES]);

    fn make_page() -> (Box<Page>, StreamSet) {
        let mut page = Box::new(Page([0; STREAM_PAGE_BYTES]));
        // SAFETY: this is a new page-aligned exclusive test backing.
        let set = unsafe { StreamSet::initialize(page.0.as_mut_ptr(), STREAM_PAGE_BYTES) }
            .expect("stream page initializes");
        (page, set)
    }

    #[test]
    fn three_rings_fit_one_page_and_transfer_partial_bytes_under_backpressure() {
        assert_eq!(REGION_BYTES, 2560);
        let (_page, set) = make_page();
        let mut writer = set.writer(Channel::Stdout).unwrap();
        let mut reader = set.reader(Channel::Stdout).unwrap();
        let source = [0x5a; STREAM_CAPACITY + 17];
        assert_eq!(writer.write(&source), Ok(STREAM_CAPACITY));
        assert_eq!(
            writer.write(&source[STREAM_CAPACITY..]),
            Err(Error::WouldBlock)
        );
        let mut first = [0; 513];
        assert_eq!(reader.read(&mut first), Ok(first.len()));
        assert_eq!(first, [0x5a; 513]);
        assert_eq!(writer.write(&source[STREAM_CAPACITY..]), Ok(17));
        let mut rest = [0; STREAM_CAPACITY];
        assert_eq!(reader.read(&mut rest), Ok(272));
        assert!(rest[..272].iter().all(|byte| *byte == 0x5a));
        assert_eq!(reader.read(&mut rest), Err(Error::WouldBlock));
    }

    #[test]
    fn counter_wraparound_preserves_fifo_order() {
        let (_page, set) = make_page();
        let mut writer = set.writer(Channel::Stderr).unwrap();
        let mut reader = set.reader(Channel::Stderr).unwrap();
        // SAFETY: only this test owns the SPSC producer and consumer.
        unsafe {
            let ring = set.ring(Channel::Stderr).as_ref();
            ring.write.store(u32::MAX - 7, Ordering::Relaxed);
            ring.read.store(u32::MAX - 7, Ordering::Relaxed);
        }
        assert_eq!(writer.write(b"wrap-around"), Ok(11));
        let mut result = [0; 11];
        assert_eq!(reader.read(&mut result), Ok(11));
        assert_eq!(&result, b"wrap-around");
    }

    #[test]
    fn close_produces_eof_after_queued_bytes_and_detects_consumer_death() {
        let (_page, set) = make_page();
        let mut writer = set.writer(Channel::Stdout).unwrap();
        let mut reader = set.reader(Channel::Stdout).unwrap();
        assert_eq!(writer.write(b"last"), Ok(4));
        writer.close();
        assert_eq!(writer.write(b"late"), Err(Error::Closed));
        let mut result = [0; 8];
        assert_eq!(reader.read(&mut result), Ok(4));
        assert_eq!(&result[..4], b"last");
        assert!(reader.writer_closed());
        assert_eq!(reader.read(&mut result), Ok(0));

        let mut writer = set.writer(Channel::Stdin).unwrap();
        let mut reader = set.reader(Channel::Stdin).unwrap();
        reader.close();
        assert_eq!(writer.write(b"peer died"), Err(Error::BrokenPipe));
    }

    #[test]
    fn helper_parent_and_child_use_opposite_ring_endpoints() {
        let (mut page, _owner) = make_page();
        // Model two address spaces: each attaches its own process-local view
        // to the same shared page, then claims opposite endpoints.
        let parent = unsafe { StreamSet::attach(page.0.as_mut_ptr(), STREAM_PAGE_BYTES) }.unwrap();
        let child = unsafe { StreamSet::attach(page.0.as_mut_ptr(), STREAM_PAGE_BYTES) }.unwrap();
        let mut to_child = parent.writer(Channel::Stdin).unwrap();
        let mut child_stdin = child.reader(Channel::Stdin).unwrap();
        let mut child_stdout = child.writer(Channel::Stdout).unwrap();
        let mut from_child = parent.reader(Channel::Stdout).unwrap();

        assert_eq!(to_child.write(b"helper-input"), Ok(12));
        let mut input = [0; 16];
        assert_eq!(child_stdin.read(&mut input), Ok(12));
        assert_eq!(&input[..12], b"helper-input");
        assert_eq!(child_stdout.write(b"helper-output"), Ok(13));
        let mut output = [0; 16];
        assert_eq!(from_child.read(&mut output), Ok(13));
        assert_eq!(&output[..13], b"helper-output");
        child_stdout.close();
        assert_eq!(from_child.read(&mut output), Ok(0));
        assert!(from_child.writer_closed());
    }

    #[test]
    fn attach_checks_layout_and_corrupt_indices_fail_closed() {
        let (mut page, _set) = make_page();
        // SAFETY: the page is the live backing initialized above.
        assert!(unsafe { StreamSet::attach(page.0.as_mut_ptr(), STREAM_PAGE_BYTES) }.is_ok());
        // SAFETY: test-only layout inspection while endpoints are idle.
        unsafe {
            let region = page.0.as_mut_ptr().cast::<Region>();
            (*region).rings[0].write.store(1000, Ordering::Relaxed);
        }
        // SAFETY: the exact initialized page remains readable and mapped.
        let set = unsafe { StreamSet::attach(page.0.as_mut_ptr(), STREAM_PAGE_BYTES) }.unwrap();
        let mut writer = set.writer(Channel::Stdin).unwrap();
        assert_eq!(writer.write(b"x"), Err(Error::Corrupt));
    }

    #[test]
    fn each_process_local_endpoint_is_unique_while_held() {
        let (_page, set) = make_page();
        let writer = set.writer(Channel::Stdout).unwrap();
        assert!(matches!(
            set.writer(Channel::Stdout),
            Err(Error::EndpointInUse)
        ));
        drop(writer);
        assert!(set.writer(Channel::Stdout).is_ok());
    }
}
