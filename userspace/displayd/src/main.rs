//! Phase-9 GOP fallback display service. All framebuffer stores run at CPL3.
//! Boot-granted slot 0 is the *only* Mmio window: immutable GOP geometry is
//! disclosed through possession-gated DISPLAY_INFO, then self-mapped RW/NX.
//! An endpoint exists for the forthcoming typed compositor protocol; until
//! it is implemented, every call fails closed and landed caps are destroyed.
#![no_std]
#![no_main]

use arena_compositor_model::wire::{self, Frame};
use arena_gfxkit::{Canvas, Rect};
mod gpu;
use core::panic::PanicInfo;
#[path = "../../abi.rs"]
mod abi;
use abi::*;

const SLOT_GOP: u64 = 0;
const SLOT_EP: u64 = 1;
const SLOT_POOL: u64 = 2;
const SLOT_DMA: u64 = 3;
const SLOT_REPLY: u64 = 9; // always free after each reply
const IPC_REFUSED: u64 = 2;
const IPC_OK: u64 = 0;

fn write_log(bytes: &[u8]) {
    unsafe {
        let _ = syscall2(SYS_DEBUG_WRITE, bytes.as_ptr() as u64, bytes.len() as u64);
    }
}
fn exit(code: u64) -> ! {
    unsafe {
        let _ = syscall1(SYS_THREAD_EXIT, code);
    }
    loop {
        core::hint::spin_loop()
    }
}
#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! {
    exit(99)
}

/// Pure arithmetic reference for the host unit test; production bars are
/// drawn by the linked toolkit. Lower 24 bits are native RGB values.
#[cfg(test)]
fn pattern(x: usize, y: usize, w: usize, h: usize) -> u32 {
    if y < h / 8 {
        return 0x00_22_33_55;
    }
    if x < w / 3 {
        0x00_e3_35_42
    } else if x < w * 2 / 3 {
        0x00_2e_c7_71
    } else {
        0x00_3b_67_e1
    }
}
fn encode(rgb: u32, fmt: u64) -> u32 {
    if fmt == 0 {
        (rgb & 0xff) << 16 | (rgb & 0x00ff00) | ((rgb >> 16) & 0xff)
    } else {
        rgb
    }
}

/// The IPC cap itself, not a wire handle or claimed PID, designates the
/// only scanout this display owns. The full, never-reused generation and
/// checked region extent come from two independent held-cap queries.
fn landed_scanout(landed: u64, owned: u64, id: u64, pages: u64) -> bool {
    if landed == CAP_NONE || landed == owned {
        return false;
    }
    let mut own = [0u64; 3];
    let mut other = [0u64; 3];
    let mut bound = [0u64; 2];
    unsafe {
        syscall2(SYS_CAP_DESCRIBE, owned, own.as_mut_ptr() as u64) == 0
            && syscall2(SYS_CAP_DESCRIBE, landed, other.as_mut_ptr() as u64) == 0
            && own[0] == 7
            && other[0] == 7
            && own[1] == id
            && other[1] == own[1]
            && (other[2] & (RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY))
                == (RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY)
            && syscall6(
                SYS_SHARED_INFO,
                landed,
                bound.as_mut_ptr() as u64,
                0,
                0,
                0,
                0,
            ) == 0
            && bound == [id, pages]
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let mut mode = [0u64; 5];
    let gop = unsafe { syscall2(SYS_DISPLAY_INFO, SLOT_GOP, mode.as_mut_ptr() as u64) } == 0;
    // Explicit boot layouts: GOP at 0, GPU at 4 when both exist; GPU at
    // 0 if firmware did not hand over a GOP. Endpoint/pool/DMA stay 1/2/3.
    let gpu_slot = if gop { 4 } else { 0 };
    let mut device = match gpu::Gpu::open(gpu_slot) {
        Ok(device) => Some(device),
        Err(reason) if gop => {
            write_log(b"[displayd] GPU fallback reason: ");
            write_log(reason.as_bytes());
            write_log(b"\n");
            None
        }
        Err(reason) => {
            write_log(b"[displayd] no valid GPU or GOP: ");
            write_log(reason.as_bytes());
            write_log(b"\n");
            exit(81)
        }
    };
    if let Some(ref gpu) = device {
        mode = [
            gpu.mode.width as u64,
            gpu.mode.height as u64,
            gpu.mode.width as u64,
            1,
            gpu.mode.span() as u64,
        ];
    }
    let [w, h, pitch, fmt, span] = mode;
    if w == 0
        || h == 0
        || w > 1024
        || h > 768
        || pitch < w
        || pitch > 1024
        || fmt > 1
        || pitch
            .checked_mul(h)
            .and_then(|v| v.checked_mul(4))
            .is_none_or(|v| v > span)
    {
        exit(82)
    }
    // Stable-boundary guest probe: this is real ring-3 memory, not an
    // allocator model. The writer loses all caps; two mappings keep its
    // zeroed physical run alive while a read-only copy cannot write-map.
    let mut created = [0u64; 3];
    if unsafe { syscall3(SYS_SHARED_CREATE, SLOT_POOL, 4, created.as_mut_ptr() as u64) } != 0 {
        exit(86)
    }
    let shared = created[0];
    if created[1] == 0 || created[2] != 4 * 4096 {
        exit(87)
    }
    let mut backing = [0u64; 3];
    if unsafe {
        syscall3(
            SYS_SHARED_PHYS,
            shared,
            SLOT_EP,
            backing.as_mut_ptr() as u64,
        )
    } != -2
        || unsafe { syscall1(SYS_CAP_PHYS, shared) } != -2
        || unsafe {
            syscall3(
                SYS_SHARED_PHYS,
                shared,
                SLOT_DMA,
                backing.as_mut_ptr() as u64,
            )
        } != 0
        || backing[0] == 0
        || backing[1] != 4
        || backing[2] != created[1]
    {
        exit(88)
    }
    let rw = unsafe { syscall2(SYS_SHARED_MAP, shared, 1) };
    if rw <= 0 {
        exit(89)
    }
    for i in [0, 4095, 4096, 12288, 16383] {
        if unsafe { core::ptr::read_volatile((rw as *const u8).add(i)) } != 0 {
            exit(90)
        }
    }
    unsafe { core::ptr::write_volatile((rw as *mut u8).add(12288), 0xa7) }
    if unsafe { syscall3(SYS_CAP_COPY, shared, 8, RIGHTS_READ | RIGHTS_DESTROY) } != 0
        || unsafe { syscall2(SYS_SHARED_MAP, 8, 1) } != -2
    {
        exit(91)
    }
    let ro = unsafe { syscall2(SYS_SHARED_MAP, 8, 0) };
    if ro <= 0 || unsafe { core::ptr::read_volatile((ro as *const u8).add(12288)) } != 0xa7 {
        exit(92)
    }
    if unsafe { syscall1(SYS_CAP_DESTROY, shared) } != 0
        || unsafe { syscall1(SYS_CAP_DESTROY, 8) } != 0
        || unsafe { core::ptr::read_volatile((ro as *const u8).add(12288)) } != 0xa7
    {
        exit(93)
    }
    write_log(b"[displayd] SharedRegion guest authority/zero/copy/mapping PASS\n");
    // Capless mappings remain pinned until an *exact own* unmap. Retire
    // this diagnostic run now, rather than silently consuming a live
    // region that production client surfaces will need later. No Rust
    // reference to its backing escapes this point.
    if unsafe { syscall6(SYS_SHARED_UNMAP, rw as u64, 0, 0, 0, 0, 0) } != 0
        || unsafe { syscall6(SYS_SHARED_UNMAP, ro as u64, 0, 0, 0, 0, 0) } != 0
        || unsafe { syscall6(SYS_SHARED_UNMAP, ro as u64, 0, 0, 0, 0, 0) } != -2
    {
        exit(101)
    }
    write_log(b"[displayd] diagnostic shared region fully unmapped\n");

    // Allocate the complete pitch, not just visible columns. A normal RAM
    // SharedRegion is a sound borrowed Canvas backing; *never* create a
    // Rust reference to the UC framebuffer MMIO mapping. The source cap and
    // mapping stay with displayd, not a client or an unimplemented compositor.
    let pixel_count = pitch.checked_mul(h).unwrap_or_else(|| exit(94));
    let bytes = pixel_count.checked_mul(4).unwrap_or_else(|| exit(94));
    let pages = bytes.div_ceil(4096);
    if pages == 0 || pages > 512 {
        exit(94)
    }
    let mut scanout = [0u64; 3];
    if unsafe {
        syscall3(
            SYS_SHARED_CREATE,
            SLOT_POOL,
            pages,
            scanout.as_mut_ptr() as u64,
        )
    } != 0
        || scanout[1] == 0
        || scanout[2] != pages * 4096
    {
        exit(95)
    }
    // Derive the actual allocation bound from the held region itself.
    // The same read-only ABI lets a future compositor validate a *received*
    // surface cap without ever seeing its physical backing.
    let mut measured = [0u64; 2];
    if unsafe {
        syscall6(
            SYS_SHARED_INFO,
            scanout[0],
            measured.as_mut_ptr() as u64,
            0,
            0,
            0,
            0,
        )
    } != 0
        || measured != [scanout[1], pages]
    {
        exit(100)
    }
    let ram = unsafe { syscall2(SYS_SHARED_MAP, scanout[0], 1) };
    if ram <= 0 {
        exit(96)
    }
    // SAFETY: the kernel minted a fresh zeroed run, mapped it RW into this
    // address space, and confirmed its size. `pixel_count * 4 <= pages *
    // 4096`; no other thread accesses it. Frame lifetime is pinned by the
    // mapping even if its cap is later removed.
    let backing = unsafe { core::slice::from_raw_parts_mut(ram as *mut u32, pixel_count as usize) };
    let mut canvas =
        Canvas::new(backing, w as usize, h as usize, pitch as usize).unwrap_or_else(|_| exit(97));
    canvas.clear(0x00_3b_67_e1);
    canvas.fill_rect(
        Rect {
            x: 0,
            y: 0,
            width: w as u32,
            height: (h / 8) as u32,
        },
        0x00_22_33_55,
    );
    canvas.fill_rect(
        Rect {
            x: 0,
            y: (h / 8) as i32,
            width: (w / 3) as u32,
            height: h as u32,
        },
        0x00_e3_35_42,
    );
    canvas.fill_rect(
        Rect {
            x: (w / 3) as i32,
            y: (h / 8) as i32,
            width: (w * 2 / 3 - w / 3) as u32,
            height: h as u32,
        },
        0x00_2e_c7_71,
    );
    // The title is drawn by the *linked no_std bitmap toolkit* in ordinary
    // RAM. A QMP font foreground sample distinguishes this from a serial
    // marker or a synthetic host-side image.
    if canvas.text(20, 20, "ARENAOS", 0x00_f8_ee_cc).is_err() {
        exit(98)
    }

    let mut gop_fb: *mut u32 = core::ptr::null_mut();
    if let Some(ref mut gpu) = device {
        let mut phys = [0u64; 3];
        if unsafe {
            syscall3(
                SYS_SHARED_PHYS,
                scanout[0],
                SLOT_DMA,
                phys.as_mut_ptr() as u64,
            )
        } != 0
            || phys[0] == 0
            || phys[1] != pages
            || phys[2] != scanout[1]
        {
            write_log(b"[displayd] GPU scanout DMA bearer failed closed\n");
            exit(83)
        }
        if let Err(reason) = gpu.paint(phys[0], phys[1]) {
            write_log(b"[displayd] GPU transport/ACK failed closed: ");
            write_log(reason.as_bytes());
            write_log(b"\n");
            exit(83)
        }
        write_log(b"[displayd] ring3 virtio-gpu 2D pixels ready (staged font v2)\n");
    } else {
        let va = unsafe { syscall2(SYS_MAP_MEMORY, SLOT_GOP, 1) };
        if va <= 0 || unsafe { syscall6(SYS_SHARED_UNMAP, va as u64, 0, 0, 0, 0, 0) } != -2 {
            exit(83)
        }
        let fb = va as *mut u32;
        gop_fb = fb;
        for y in 0..h as usize {
            for x in 0..w as usize {
                let color = encode(backing[y * pitch as usize + x], fmt);
                unsafe { core::ptr::write_volatile(fb.add(y * pitch as usize + x), color) }
            }
        }
        write_log(b"[displayd] ring3 GOP pixels ready (staged font v2)\n");
    }
    // Bounded GRAPHICS v1 display protocol. Possession of the endpoint
    // admits MODE; a PRESENT additionally requires a matching transferred
    // *region cap*. A wire handle, guessed generation or sender PID never
    // substitutes for that cap. Production grants will give the endpoint
    // only to the compositor; this first bridge has no client yet.
    let mut msg = [0u64; 3];
    let mut payload = [0u8; wire::BYTES];
    let mut answer = [0u8; wire::BYTES];
    loop {
        let r = unsafe {
            syscall3(
                SYS_IPC_RECV,
                SLOT_EP,
                msg.as_mut_ptr() as u64,
                payload.as_mut_ptr() as u64,
            )
        };
        if r < 0 {
            exit(84)
        }
        let frame = Frame::decode(&payload);
        let mut status = IPC_REFUSED;
        let mut result = 0u64;
        let mut reply_cap = CAP_NONE;
        let mut echo = Frame::Mode;
        if msg[0] == 0 && msg[1] == 0 {
            match frame {
                Ok(Frame::Mode) if msg[2] == CAP_NONE => {
                    // Attenuate a local copy before sending it. This exact
                    // cap remains displayd-owned; the IPC queue keeps its
                    // own staged reference even after SLOT_REPLY is freed.
                    if unsafe {
                        syscall3(
                            SYS_CAP_COPY,
                            scanout[0],
                            SLOT_REPLY,
                            RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY | RIGHTS_DESTROY,
                        )
                    } != 0
                    {
                        exit(102)
                    }
                    reply_cap = SLOT_REPLY;
                    status = IPC_OK;
                    result = w | (h << 32);
                }
                Ok(Frame::Present { x, y, w: rw, h: rh })
                    if landed_scanout(msg[2], scanout[0], scanout[1], pages)
                        && x >= 0
                        && y >= 0
                        && u64::from(x as u32)
                            .checked_add(u64::from(rw))
                            .is_some_and(|end| end <= w)
                        && u64::from(y as u32)
                            .checked_add(u64::from(rh))
                            .is_some_and(|end| end <= h) =>
                {
                    let rect = arena_gpu2d_wire::Rect {
                        x: x as u32,
                        y: y as u32,
                        width: u32::from(rw),
                        height: u32::from(rh),
                    };
                    if let Some(ref mut gpu) = device {
                        if let Err(reason) = gpu.refresh(rect) {
                            write_log(b"[displayd] GPU PRESENT failed closed: ");
                            write_log(reason.as_bytes());
                            write_log(b"\n");
                            exit(103)
                        }
                    } else {
                        if gop_fb.is_null() {
                            exit(103)
                        }
                        let src = ram as *const u32;
                        for row in y as usize..(y as usize + rh as usize) {
                            let start = row * pitch as usize + x as usize;
                            if fmt != 0 {
                                // Identity format: one row copy. SAFETY: the
                                // validated rectangle lies in both the pinned
                                // scanout RAM and the mapped GOP framebuffer;
                                // raw pointers create no Rust alias, and the
                                // only writer (the compositor) is blocked in
                                // this synchronous PRESENT call.
                                unsafe {
                                    core::ptr::copy_nonoverlapping(
                                        src.add(start),
                                        gop_fb.add(start),
                                        rw as usize,
                                    )
                                }
                                continue;
                            }
                            for at in start..start + rw as usize {
                                // SAFETY: as above; swizzled formats need a
                                // per-pixel encode.
                                unsafe {
                                    let color = core::ptr::read_volatile(src.add(at));
                                    core::ptr::write_volatile(gop_fb.add(at), encode(color, fmt));
                                }
                            }
                        }
                    }
                    status = IPC_OK;
                    echo = Frame::Present { x, y, w: rw, h: rh };
                }
                _ => (),
            }
        }
        // The first-free landed slot is not stable. Dispose of even an
        // unexpected/invalid cap, before answering any caller. This is
        // harmless to the sender's independent copy and prevents capacity
        // exhaustion under repeated hostile requests.
        if msg[2] != CAP_NONE && unsafe { syscall1(SYS_CAP_DESTROY, msg[2]) } != 0 {
            exit(85)
        }
        if echo.encode(&mut answer).is_err() {
            exit(104)
        }
        let replied = unsafe {
            syscall5(
                SYS_IPC_REPLY,
                SLOT_EP,
                status,
                result,
                reply_cap,
                answer.as_ptr() as u64,
            )
        };
        if reply_cap != CAP_NONE && unsafe { syscall1(SYS_CAP_DESTROY, SLOT_REPLY) } != 0 {
            exit(105)
        }
        // A caller can die between RECV and REPLY; its cancelled call is
        // a typed refusal, not permission to retain a staged reference.
        if replied != 0 && replied != -2 {
            exit(106)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pattern_bounded_and_channel_order() {
        assert_eq!(pattern(0, 0, 800, 600), 0x00223355);
        assert_eq!(pattern(0, 100, 800, 600), 0x00e33542);
        assert_eq!(pattern(400, 100, 800, 600), 0x002ec771);
        assert_eq!(pattern(799, 599, 800, 600), 0x003b67e1);
        assert_eq!(encode(0x00e33542, 0), 0x004235e3);
    }
}
