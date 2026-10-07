# ADR-0098: Headless installed application launch

**Status:** Accepted for Phase 13; helper processes and multi-process instance
ownership remain open.

## Context

APB1 manifests already carry a descriptive `FLAG_HEADLESS` policy and the
Startup ABI v2 validates it. The production Desktop launcher rejected that
flag because every launch allocated a SharedRegion surface, authenticated
Desktop endpoint, and ordinary window. This made signed background work
impossible and gave a no-window program authority it did not need.

## Decision

An installed headless launch uses the same fresh receiver-verified Image
capability as an ordinary installed launch. The ID, manifest flags, package
path, and executable name remain descriptive policy inputs. `packaged` rebuilds
the current protected APB1 catalog, rechecks signer eligibility and the exact
installed tree, and returns the execution capability only after verification.

Desktop reserves one bounded instance slot and a nonzero monotonic startup
generation, creates the one-page read-only startup block, and spawns through
its existing exact Process-capability group. The child receives only:

1. Startup ABI v2 in the kernel-designated startup slot; and
2. its private Notification capability attenuated to `READ|WRITE` so it can
   wait and arm a timer.

It receives no Desktop endpoint, surface SharedRegion, document File cap,
filesystem root, or window handle. `stdin`, `stdout`, and `stderr` remain
absent until the native byte-stream workstream supplies real stream endpoints.
A headless document-open request is refused before launch.

The Desktop tracks the instance as a process-owning record with no window.
Liveness and reap use the exact held Process capability. Exited children pass
through the ordinary sweep and release their instance record. A non-multi-
instance application that is already live is not launched a second time.

The high bit in the headless record's internal descriptive key separates it
from the small SharedRegion IDs used by the existing window-owner model. It is
never accepted as authority. The ABI instance slot and generation are likewise
descriptive; only the Process cap authorizes liveness and reap.

## Consequences

- Signed installed applications can run without creating a compositor window
  or receiving Desktop session authority.
- The headless launch consumes one application-instance slot, one process
  record, one Process cap in the manager, and the child Notification cap. The
  startup SharedRegion is destroyed by the startup runtime before application
  code runs.
- This does not yet provide helper-role allowlists, multi-process
  `AppInstance` ownership, standard streams, or manager-death policy. Those
  require separate lifecycle and stream work.
- APB1 `FLAG_HEADLESS` is a signed policy attribute, not a capability. Only the
  trusted launcher interprets it, and only a verified Image cap can be spawned.

## Evidence

`tools/test_phase13_registry_guest.py` installs a signed headless APB1 package
through the normal Desktop and filesd path, selects it from All Applications,
and observes the child validate Startup ABI v2 against its complete cap table.
While active it owns a held Process cap and timer, adds no ordinary window,
SharedRegion, region page, or mapping, and has no inherited surface or Desktop
endpoint. It waits on a real timer, exits, is reaped through the held Process
cap, and returns the process, region, page, map, and capability inventory to
baseline.
