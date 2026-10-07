# ADR-0104: Native byte streams over bounded SharedRegion rings

## Status

Accepted for Phase 13.

## Context

Native Startup ABI v2 already reserves stdin, stdout, and stderr descriptor
references, but no native stream mechanism exists. ArenaOS has bounded
SharedRegions, per-AppInstance wait notifications, a Desktop event notification,
and exact attenuating spawn grants. A kernel pipe subsystem would duplicate
those primitives and complicate process teardown.

The 16-instance / 32-window Phase-13 pressure workload needs additional
SharedRegion and mapping records if each stream set is separately backed. Its
resource budget is approximately 82 live SharedRegion records and 132 live
shared mappings, including the existing 2-region / 4-map boot baseline. The
current limits are 80 and 128.

## Options considered

- Add kernel stream objects and stream syscalls. This would create a new
  kernel object lifecycle and cancellation surface for a facility that can be
  expressed with existing shared memory and notifications.
- Put streams in the graphical surface region. A surface grant would then also
  be the stream grant, and headless applications would need a different
  contract.
- Use one separately held SharedRegion containing bounded single-producer,
  single-consumer rings, with notification capabilities used only as wake
  hints. This keeps data and authority explicit and also works for headless
  applications and helpers.

## Decision

An opt-in installed application declares `FLAG_STANDARD_STREAMS` in its
receiver-verified APB1 application manifest. This signed bit requests a
standard stream set; it is descriptive policy, not authority. The trusted
Desktop creates a zeroed one-page SharedRegion and passes its exact attenuated
READ|WRITE cap in Startup ABI v2 under a `StandardStreamSet` role. Startup's
stdin, stdout, and stderr fields all refer to that one descriptor. The set
contains three independent single-producer, single-consumer rings, each with
768 bytes of usable capacity. Atomic producer/consumer counters use
acquire/release ordering. Read and write operations return partial counts,
`WouldBlock`, EOF, or a broken-peer result; close is explicit. The producer
and consumer each send a notification wake hint after changing a condition
that may unblock the peer.

The standard input producer is Desktop. It forwards printable input to the
focused stream-enabled AppInstance. The application is the producer for
stdout and stderr; Desktop drains those rings to its bounded native log sink.
Desktop signals the app's existing private AppInstance notification when
stdin becomes available or output space is reclaimed. The application gets a
WRITE-only cap to the existing Desktop event notification so stream writes
wake the broker. This is a wake hint only; all stream state remains in the
held SharedRegion capability.

Desktop retains the exact stream SharedRegion cap and mapping in the
AppInstance record. It closes the appropriate ring side when the primary or
AppInstance exits, drains final output before teardown, then unmaps and
destroys the exact owner cap. A dead or closed peer cannot leave a waiter
parked on an unobservable transition because every state change is published
before its wake hint.

The existing 16-instance / 32-window model calculation exceeds the current
SharedRegion and mapping record limits by two and four records respectively.
Raise those bounded tables to 96 SharedRegions and 160 mappings, preserving the
36,864-page aggregate budget. The 16-instance fixture will measure actual
high-water and exact teardown before these new limits are treated as qualified
capacity. The per-Process spawn inheritance bound increases from five to seven,
including the Startup transport cap in slot 0. The Startup ABI descriptor
bound increases from four to six to fit the explicitly opted-in stream region,
stream wake cap, and an existing optional tail grant. The 128 capability slots
and slot-127 observability remain unchanged.

## Reasoning

The SharedRegion cap is the data authority. Application IDs, channel numbers,
window handles, and startup field references only select a channel within the
held region. No stream grant includes a filesystem root, Desktop endpoint,
Image, Process cap, or any other authority. The notification caps only cause
the owning process's wait loop to recheck the ring. EOF and peer death are
ordinary bounded ring-state transitions managed by Desktop's exact
ProcessGroup ownership.

The 768-byte ring capacity makes three rings and their aligned metadata fit in
one page with unused canonical padding. This is deliberately small; producers
must handle partial transfers and backpressure instead of assuming unbounded
buffering.

## Downsides accepted

These are cooperative single-producer, single-consumer channels. A process
holding a SharedRegion can corrupt its own stream contents or indices; this
does not grant access outside that exact region. The Desktop log sink is
bounded by the ring and drained incrementally. It is not a persistent log
file. The stream feature is opt-in so existing APB1 applications keep their
Phase-12 Startup ABI inventory.

## Future implications

Signed helper declarations may request a dedicated stream set. Desktop must
share that exact ring cap with the allowlisted helper and its owner, retain
exact ProcessGroup lifecycle, and wake blocked participants only through
explicitly granted notification authority. Shell pipelines can connect two
held ring endpoints in userspace without changing the stream wire or adding
POSIX syscall semantics.
