//! Signed <=4096-byte hostile file-capability probe (Phase 11.6, ADR-0077).
//! Not a boot image. An untrusted application that asks the trusted
//! chooser for one document read-only, then tries to do more with it
//! than the user granted, through raw filesd requests: write, truncate,
//! delete, name `..` or `System`, amplify rights, forge a session page,
//! a destination or a request. It paints its window green when every
//! answer is the expected one, red (with the first unexpected check as a
//! dark band) otherwise, and blue when the user cancelled.
#![no_std]
#![no_main]
#[path = "../../../userspace/abi.rs"]
mod abi;
use abi::*;
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    done(99)
}
fn done(code: u64) -> ! {
    unsafe { syscall1(SYS_THREAD_EXIT, code) };
    loop {
        core::hint::spin_loop()
    }
}
/// One call through `slot` lending `cap`: (status, value, landed cap).
#[inline(never)]
fn call(slot: u64, bytes: &mut [u8; 64], cap: u64) -> [u64; 3] {
    let mut out = [u64::MAX, 0, CAP_NONE];
    if unsafe {
        syscall6(
            SYS_IPC_CALL,
            slot,
            0,
            0,
            cap,
            out.as_mut_ptr() as u64,
            bytes.as_mut_ptr() as u64,
        )
    } != 0
    {
        out[0] = u64::MAX;
    }
    out
}
#[inline(never)]
fn magic(m: &[u8; 4], op: u8) -> [u8; 64] {
    let mut b = [0u8; 64];
    b[0] = m[0];
    b[1] = m[1];
    b[2] = m[2];
    b[3] = m[3];
    b[4] = 1;
    b[5] = op;
    b
}
/// A desktop frame for window `handle`.
#[inline(never)]
fn frame(op: u8, handle: u64) -> [u8; 64] {
    let mut b = magic(b"ADSK", op);
    unsafe { core::ptr::write_unaligned(b.as_mut_ptr().add(8).cast::<u64>(), handle) };
    b
}
/// A filesd request (`filesd_wire.rs`): op, rights, offset, len, name.
#[inline(never)]
fn req(op: u8, rights: u8, offset: u64, len: u32, name_len: u16) -> [u8; 64] {
    let mut b = [0u8; 64];
    b[0] = b'A';
    b[1] = b'F';
    b[2] = b'2';
    b[3] = b'Q';
    b[4] = op;
    b[5] = rights;
    unsafe {
        core::ptr::write_unaligned(b.as_mut_ptr().add(8).cast::<u64>(), offset);
        core::ptr::write_unaligned(b.as_mut_ptr().add(16).cast::<u32>(), len);
        core::ptr::write_unaligned(b.as_mut_ptr().add(20).cast::<u16>(), name_len);
    }
    b
}
#[inline(never)]
fn paint(va: u64, color: u32, band: usize) {
    for y in 0..60 {
        for x in 0..80 {
            let v = if band != 0 && y >= band * 4 && y < band * 4 + 4 {
                0xff20_0000
            } else {
                color
            };
            unsafe { core::ptr::write_volatile((va as *mut u32).add(y * 80 + x), v) }
        }
    }
}
fn show(va: u64, handle: u64, color: u32, band: usize) {
    paint(va + 4096, color, band);
    if call(0, &mut frame(2, handle), 1)[0] != 0 {
        done(88)
    }
}
/// Write `s` at the start of the I/O page.
fn put(io: u64, s: &[u8]) {
    for (i, b) in s.iter().enumerate() {
        unsafe { core::ptr::write_volatile((io as *mut u8).add(i), *b) };
    }
}
const S_OK: u64 = 0;
const S_INVAL: u64 = 6;
const S_DENIED: u64 = 12;
#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let va = unsafe { syscall2(SYS_SHARED_MAP, 1, 1) };
    if va <= 0 {
        done(83)
    }
    let va = va as u64;
    let mut create = frame(1, 0);
    create[24] = 80;
    create[26] = 60;
    let out = call(0, &mut create, 1);
    if out[0] != 0 || out[1] == 0 {
        done(84)
    }
    let handle = out[1];
    let mut title = frame(4, handle);
    title[32..43].copy_from_slice(b"Files Probe");
    if call(0, &mut title, 1)[0] != 0 {
        done(85)
    }
    show(va, handle, 0xff80_8080, 0);
    // Ask the user for one document, read-only.
    let mut choose = magic(b"ASVC", 13);
    choose[9] = 1;
    if call(0, &mut choose, 1)[0] != 0 {
        done(86)
    }
    // Wait for the outcome (or a close).
    loop {
        let mut event = frame(3, handle);
        if call(0, &mut event, 1)[0] != 0 {
            done(89)
        }
        if event[5] == 5 && event[28] == 3 {
            done(42)
        }
        if event[5] == 5 && event[28] == 10 {
            break;
        }
        if unsafe { syscall3(SYS_TIMER_ARM, 3, 1, 20_000) } < 0
            || unsafe { syscall1(SYS_WAIT, 3) } != 1
        {
            done(91)
        }
    }
    let mut take = magic(b"ASVC", 15);
    let got = call(0, &mut take, 1);
    let f = got[2];
    if f == CAP_NONE {
        // Cancelled: nothing was granted.
        show(va, handle, 0xff20_40c0, 0);
    } else {
        let pages = unsafe { syscall1(SYS_SHARED_PAGES, 1) } as u64;
        let io = va + (pages - 1) * 4096;
        let mut bad = 0usize;
        let mut k = 0usize;
        let mut expect = |r: u64, want: u64| {
            k += 1;
            if r != want && bad == 0 {
                bad = k;
            }
        };
        // 1 The grant is read-only (Granted, read_only set).
        expect(u64::from(take[5] == 16 && take[9] == 1), 1);
        // 2 A session page outside the region is refused.
        expect(call(f, &mut req(1, 0, pages + 7, 0, 0), 1)[0], S_INVAL);
        // 3 The real last page is accepted.
        expect(call(f, &mut req(1, 0, pages - 1, 0, 0), 1)[0], S_OK);
        // 4 Reading the granted document works.
        expect(call(f, &mut req(7, 0, 0, 16, 0), CAP_NONE)[0], S_OK);
        // 5 Writing it does not.
        expect(call(f, &mut req(8, 0, 0, 4, 0), CAP_NONE)[0], S_DENIED);
        // 6 Nor truncating it.
        expect(call(f, &mut req(9, 0, 0, 0, 0), CAP_NONE)[0], S_DENIED);
        // 7 `..` from the document reaches nothing.
        put(io, b"..");
        expect(call(f, &mut req(4, 63, 0, 0, 2), CAP_NONE)[0], S_DENIED);
        // 8 Nor does naming System.
        put(io, b"System");
        expect(call(f, &mut req(2, 0, 0, 0, 6), CAP_NONE)[0], S_DENIED);
        // 9 Nor deleting a sibling.
        expect(call(f, &mut req(10, 0, 0, 0, 6), CAP_NONE)[0], S_DENIED);
        // 10 Re-opening itself with all rights gives no more than READ.
        let c2 = call(f, &mut req(4, 63, 0, 0, 0), CAP_NONE);
        expect(c2[0], S_OK);
        expect(call(c2[2], &mut req(8, 0, 0, 4, 0), CAP_NONE)[0], S_DENIED);
        // 12 A rename lending a non-filesd capability as the destination.
        put(io, b"xy");
        expect(call(f, &mut req(12, 0, 0, 1, 1), 1)[0], S_DENIED);
        // 13 A forged request (bad magic) is refused.
        let mut forged = req(7, 0, 0, 16, 0);
        forged[0] = b'X';
        expect(call(f, &mut forged, CAP_NONE)[0], S_INVAL);
        // 14 Revoking with something that is no filesd record.
        expect(call(f, &mut req(15, 0, 0, 0, 0), 1)[0], S_DENIED);
        if bad == 0 {
            show(va, handle, 0xff20_a040, 0);
        } else {
            show(va, handle, 0xffc0_2020, bad);
        }
    }
    loop {
        let mut event = frame(3, handle);
        if call(0, &mut event, 1)[0] != 0 {
            done(89)
        }
        if event[5] == 5 && event[28] == 3 {
            done(42)
        }
        if unsafe { syscall3(SYS_TIMER_ARM, 3, 1, 20_000) } < 0
            || unsafe { syscall1(SYS_WAIT, 3) } != 1
        {
            done(91)
        }
    }
}
