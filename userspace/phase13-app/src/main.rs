#![no_std]
#![no_main]

use arena_desktop::{
    app_client,
    client::{self, Client},
    files::Files,
    model::Event,
};
use arena_startup_abi::startup::StartupView;

const APPLICATION_ID: [u8; 32] = {
    let mut id = [0u8; 32];
    let bytes = b"org.arenaos.phase13app";
    let mut index = 0;
    while index < bytes.len() {
        id[index] = bytes[index];
        index += 1;
    }
    id
};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    client::exit(99)
}

arena_desktop::entry!(main, 64 * 1024);

extern "C" fn main() -> ! {
    match arena_runtime::startup::run(|view| application_main(view)) {
        Ok(()) | Err(_) => client::exit(70),
    }
}

fn application_main(view: StartupView<'_>) -> ! {
    let actual_entry = _start as *const () as usize as u64;
    let has_document = view.capability_count() == 4;
    if view.application_id() != &APPLICATION_ID
        || view.argument_count() != 1
        || view.argument(0) != Some(b"org.arenaos.phase13app")
        || !matches!(view.capability_count(), 3 | 4)
        || view.entry() != actual_entry
        || view.load_base() != 0x0020_0000
        || view.instance_generation() == 0
    {
        client::exit(71);
    }
    let (kind, dark, motion, path) = match app_client::startup() {
        Ok(startup) => startup,
        Err(error) => client::exit(80 + error.unsigned_abs() % 20),
    };
    if kind != 6 || dark || !motion || path != [0; 32] {
        client::exit(73);
    }
    app_client::audit(kind, view.instance_generation()).unwrap_or_else(|_| client::exit(74));

    let window = Client::connect_v2(320, 180, "Phase13 Probe").unwrap_or_else(|_| client::exit(75));
    if has_document {
        let descriptor = view.capability(3).unwrap_or_else(|| client::exit(81));
        let mut actual = [0u64; 3];
        if unsafe { app_client::describe_raw(4, &mut actual) } != 0
            || descriptor.kind != 12
            || !view.capability_matches(3, actual)
        {
            client::exit(82);
        }
        let (page, base) = window.file_page();
        let files = Files::session(4, window.backing_slot(), base, page)
            .unwrap_or_else(|_| client::exit(83));
        let mut document = [0u8; 128];
        let count = files
            .read(4, 0, &mut document)
            .unwrap_or_else(|_| client::exit(84));
        if &document[..count] != b"Phase 13 associated document\n"
            || files.write(4, 0, b"must remain read-only").is_ok()
        {
            client::exit(85);
        }
        client::log(b"[phase13-app] exact read-only document capability verified\n");
    }
    for y in 0..window.height {
        for x in 0..window.width {
            let color = if (x / 32 + y / 24) & 1 == 0 {
                0x286c_9c
            } else {
                0x3b91_68
            };
            unsafe { window.pixels.add(y * window.width + x).write(color) };
        }
    }
    window.damage().unwrap_or_else(|_| client::exit(76));
    if !has_document {
        client::log(b"[phase13-installed-app] ABI-v2 startup verified; real window published\n");
    }

    loop {
        app_client::idle(None).unwrap_or_else(|_| client::exit(77));
        loop {
            match window.poll().unwrap_or_else(|_| client::exit(78)) {
                Some(Event::Close) => client::exit(42),
                Some(_) | None => {}
            }
            if !window.more.get() {
                break;
            }
        }
    }
}
