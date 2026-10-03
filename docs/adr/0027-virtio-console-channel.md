# ADR-0027 — The virtio-console channel: `consoled`, and what makes a port a console

*Milestone 6.4. Status: accepted.*

## Problem

Phase 6's fourth driver was to be `virtio-console`: the roadmap said
"a second console channel (virtio-serial port) as a userspace service:
the shell's stdout/stdin can live on it, serial stays the kernel's
panic/diagnostic path."

That sentence hides the actual question. Moving bytes over a
virtio-serial port is a morning's work on the shared virtio core
(ADR-0025) — it is two queues and no protocol. The question is what
those bytes are *connected to*. A console is not a byte pipe: it is a
line discipline (echo, backspace, line commit, a blocking reader) on
the way in, and everything the machine chooses to print on the way
out. ArenaOS already has exactly one of each, in the kernel, serving
the serial port and — since ADR-0026 — a keyboard.

So: does a second channel get its own console, or a share of the one
that exists?

## Approaches considered

**A. The shell's stdout/stdin move to consoled over IPC.** The
roadmap's literal reading. Rejected on the same grounds ADR-0026
rejected its predecessor, and more strongly. It makes the shell depend
on a driver that may not exist; it duplicates the line discipline in
userspace (two editors, two backspace implementations, two definitions
of "a line"); it breaks every headless boot and every automated test;
and it answers "which console is the real one?" with "whichever
started first".

**B. consoled gets its own private line discipline and terminal.** A
port would then be a *different* console: type on serial and type on
the port, get two independent sessions. Honest, and eventually right
for a multi-session OS — but v1 has one shell, and two consoles onto
one shell means two readers racing for one input queue, which the
kernel's one-reader reservation exists to forbid. Deferred, with the
note that Phase 8's multi-session work is where it belongs.

**C. The kernel keeps the console; drivers attach CHANNELS to it.**
Chosen. The console stays one object with one line discipline and one
reader. A channel is a pair of taps on it:

- **inbound** — `SYS_CONSOLE_PUSH` (23), gated on `CapObj::ConsoleInput`,
  already built for the keyboard: decoded bytes enter the *same*
  discipline the UART's RX ISR feeds.
- **outbound** — new: `SYS_CONSOLE_ATTACH` (24) and
  `SYS_CONSOLE_PULL` (25), gated on a new `CapObj::ConsoleOutput`
  singleton, over a kernel-side ring that MIRRORS every byte the
  console emits.

The shell needed no changes. Serial needed no changes. A machine with
no console device behaves exactly as it did.

## Decision

`userspace/consoled` (spawn-registry image 12) is the fifth driver on
the shared virtio core and the first with a queue in each direction.

**Transport: `virtio-serial-pci` + a `virtconsole` port, with
`VIRTIO_CONSOLE_F_MULTIPORT` deliberately NOT negotiated.** Declining
the bit is not laziness — it is what makes the driver small. QEMU's
`set_status` marks port 0 `guest_connected` as soon as a non-multiport
guest reaches DRIVER_OK (`hw/char/virtio-serial-bus.c`), so one port
works in both directions with two queues, no control queues, no port
discovery protocol, and no open/close events. `SIZE` is declined
because there is no terminal to resize; `EMERG_WRITE` because ArenaOS
panics on the serial port, which is the kernel's own and always up —
a panic path through a ring-3 driver would be a worse one. The fixture
pins `max_ports=1`, at which QEMU does not even offer MULTIPORT.

**The output mirror.** One holder of `ConsoleOutput` attaches a
notification (nid + badge, exactly like an IRQ relay). From then on
every byte the kernel writes to the console is also appended to a
4 KiB ring, and the attacher is woken when the ring goes from empty to
non-empty. It drains with `SYS_CONSOLE_PULL` and writes the bytes
wherever it likes. Full = drop the oldest and count it: the kernel's
own output must never stall on a userspace channel that stopped
draining. Off by default, off again when the attacher dies (swept in
`proc::destroy`, beside relay vectors).

**Two capabilities, not one.** `ConsoleInput` and `ConsoleOutput` are
separate objects granted separately. A keyboard driver holds the first
and never the second, so it cannot read everything the machine prints;
a log sink could hold the second and never gain the power to forge
input. Exactly one process holds both — the production `consoled` —
because a console is both halves, and the kernel says so by handing
over both.

**Mode selection is two capability probes** (ADR-0026's pattern): a
zero-length push and an attach. The m6 suite withholds both, so the
same image serves the port over IPC instead, and the switch is under
test as much as the device is.

**Buffers are posted AFTER DRIVER_OK** — the opposite of inputd's
order, for a device-specific reason. QEMU asks `chr_can_read` before
handing a port any host bytes, and that answers 0 while the guest is
not yet DRIVER_OK; a chardev whose frontend says "cannot read" is
PAUSED, and only `handle_input` → `guest_writable` → `accept_input`
resumes it — which needs the port already `guest_connected`. Buffers
posted before the status bit therefore leave the host side paused
forever: the first thing anyone types goes nowhere.

## Two latent kernel bugs this milestone exposed

Neither was caused by 6.4. Both had been in the kernel for
milestones, waiting for something to change *when* interrupts land.

**The relay interrupt stubs clobbered `rcx` before saving it**
(since M5.1). Each stub did `mov rcx, <vector>` and then jumped to a
common half that pushed `rcx` — by which time the interrupted thread's
value was already gone, and the matching `pop rcx` handed it back the
*vector number*. An interrupt may land on any instruction and `rcx` is
a perfectly legal place for live data (it is Win64's first argument
register). The symptom: a spawned thread's record index, read
correctly and logged correctly as 6, arrived at its callee as 54 —
the number of the relay vector a console channel had just started
using. The vector now travels on the stack and is read back after the
saves, exactly as `exception_common` has always done. Nothing but a
new interrupt source firing during process spawn would have found it.

**Boot-time TSC calibration halted the machine on a busy host.**
`timekeeping::init` requires two PIT windows to agree within 5% and
halts otherwise. On a virtual machine the TSC advances with the HOST's
clock while the PIT advances with virtual time, so a host stall of a
couple of milliseconds inside a 20 ms window reads as a 10% error:
2860 MHz measured against a true 2603. That is a boot failure with no
cause inside the machine at all — a user on a loaded laptop could
simply fail to boot. `init` now takes up to three rounds and accepts
the first agreeing pair; the m2 suite's cross-check does the same. A
genuinely unstable or unreadable TSC disagrees in every round and
still halts, which is the invariant worth keeping. Healthy windows
agree to about 0.04%; disturbed ones are off by 10% or more — there is
nothing in between to be ambiguous about, and both now log their
margin in ppm.

## Reasoning

The console is a kernel object because the line discipline is policy
the kernel already owns and the panic path must never depend on a
userspace driver. Channels are ring-3 because device protocols are
where drivers belong. That split is the same one storaged, netd, rngd,
and inputd already live on, and it is why this milestone added one
kernel mechanism (a mirror) rather than a subsystem.

Declining MULTIPORT is the ADR-0024 discipline applied again: the
smallest device contract that does the job, with the reason it is
sufficient written down against the device's own source.

## Downsides accepted

- **One channel at a time.** `attach_output` refuses a second holder
  (`STATUS_BUSY`), mirroring the single-reader rule on the input side.
  Multi-session consoles are Phase 8's problem, not a v1 shape.
- **The mirror can drop.** A channel that stops draining loses the
  oldest bytes, counted and visible in the stats. The alternative —
  back-pressuring the kernel's own output — is worse in every case,
  including the panic path.
- **Up to one tick of latency** (10 ms) between a byte reaching the
  console and the channel being woken. The wake cannot be issued from
  the tap: `serial::putc` runs in every context this kernel has,
  including inside the scheduler's own log lines, and waking a thread
  from underneath the scheduler re-enters it. The tap therefore only
  appends and raises a flag; the timer's auxiliary hook pays the wake.
  The first version notified from the tap and wedged the machine
  mid-boot.
- **A feature nobody uses must still cost nothing.** The tap's inert
  path is three inlined instructions behind one relaxed load. It was
  briefly an out-of-line call, and on a boot's tens of thousands of
  console bytes that was enough emulated work to fail the m2 suite's
  clock calibration one run in three. Measured, bisected, fixed.
- **The harness has a third actor.** A serial feeder, a QMP typist,
  and now a socket peer. The fixture chardev uses `wait=on` so QEMU
  does not start until the actor is attached — QEMU discards a console
  port's output while nobody is connected, and a race there produced
  honest-but-useless SKIPs about one boot in five. Users get
  `wait=off`: their machine must boot whether or not anyone connects,
  and that configuration is itself tested.

## Future implications

- `SYS_CONSOLE_PULL` is the first kernel→userspace *stream* in ArenaOS.
  A log service, a serial-over-network shell, and the desktop phase's
  terminal all want the same shape; if a second consumer ever needs it
  concurrently, the single-attacher rule is what has to give, and the
  mirror becomes per-channel.
- **The roadmap's open question — does the desktop terminal multiplex
  over consoled, or wait for GPU text?** Decided here: it waits.
  consoled is a host-facing operations channel (a pipe to `nc -U`, a
  serial line, a log collector), and modelling a graphical terminal as
  a chardev would inherit every limitation of one — no resize, no
  scrollback, no multiple sessions, one reader. The desktop terminal
  will be a compositor client over the future text/GPU path, and will
  talk to a *session* object, not to this driver.
- Registry images 12 and 13 leave four spare of 16. The next driver
  pair fits; the one after that raises the bound again.
- The relay-stub fix is a reminder to audit interrupt entry paths
  whenever a new vector source appears — the bug was invisible until
  something fired at an unlucky moment, and the next such bug will be
  too.
