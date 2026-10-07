#![no_std]
#![no_main]

use arena_desktop::client;
use arena_lib::abi::{
    RIGHTS_READ, RIGHTS_WRITE, SYS_CAP_DESCRIBE, SYS_TIMER_ARM, SYS_WAIT, syscall1, syscall2,
    syscall3,
};
use arena_startup_abi::manifest::FLAG_HEADLESS;
use arena_startup_abi::startup::{CAP_KIND_NOTIFICATION, StartupView};

const APPLICATION_ID: [u8; 32] = {
    let mut id = [0; 32];
    let bytes = b"org.arenaos.zzheadless";
    let mut index = 0;
    while index < bytes.len() {
        id[index] = bytes[index];
        index += 1;
    }
    id
};
const COMPLETION_BADGE: u64 = 0x13A1;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    client::exit(99)
}

arena_desktop::entry!(main, 16 * 1024);

extern "C" fn main() -> ! {
    match arena_runtime::startup::run(application_main) {
        Ok(()) | Err(_) => client::exit(70),
    }
}

fn application_main(view: StartupView<'_>) {
    let mut observed = [0u64; 3];
    if view.application_id() != &APPLICATION_ID
        || view.argument_count() != 1
        || view.argument(0) != Some(b"org.arenaos.zzheadless")
        || view.flags() & FLAG_HEADLESS == 0
        || view.instance_generation() == 0
        || view.capability_count() != 1
        || view.stdin_descriptor().is_some()
        || view.stdout_descriptor().is_some()
        || view.stderr_descriptor().is_some()
        || view.capability(0).is_none_or(|descriptor| {
            descriptor.slot != 1
                || descriptor.kind != CAP_KIND_NOTIFICATION
                || u64::from(descriptor.rights) != (RIGHTS_READ | RIGHTS_WRITE)
        })
        || unsafe { syscall2(SYS_CAP_DESCRIBE, 1, observed.as_mut_ptr() as u64) } != 0
        || observed[0] != u64::from(CAP_KIND_NOTIFICATION)
        || observed[2] != (RIGHTS_READ | RIGHTS_WRITE)
    {
        client::exit(71);
    }

    client::log(b"[phase13-headless] Startup ABI v2 verified one attenuated Notification; no window caps present\n");
    let timer = unsafe { syscall3(SYS_TIMER_ARM, 1, COMPLETION_BADGE, 3_000_000) };
    if timer < 0 || unsafe { syscall1(SYS_WAIT, 1) } != COMPLETION_BADGE as i64 {
        client::exit(72);
    }
    client::log(b"[phase13-headless] timer completed; process exiting for manager reap\n");
    client::exit(42)
}
