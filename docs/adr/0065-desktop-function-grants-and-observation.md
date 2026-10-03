# ADR-0065: Desktop function grants, pacing and read-only observation

Status: implemented prototype; six-app guest gate green, adversarial qualification pending.

Ordinary graphical programs receive no raw filesystem, display, input, Power,
registrar or unrelated Process capability. The userspace desktop broker owns
each fresh SharedRegion and the original Process/READ|DESTROY returned by
SYS_SPAWN. It binds both before serving any child request. Every operation
checks the full backing generation and held original-child liveness.

Two distinct references to that backing serve different roles. The graphical
reference has READ|WRITE|COPY. A function reference explicitly includes DESTROY
as a service marker plus READ and/or WRITE and COPY. This is an attenuated copy
of a held object, not a newly minted kernel identity. Its native destruction
only drops that holder's reference. The broker additionally checks the resource
scope provisioned when it allocated the session: file read, file write,
preferences, launch, or none. Rights attenuation cannot enlarge that scope.
No filename, startup application kind, PID, title or window number grants it.
Delegation follows possession; an original-child death invalidates all proxy
operations even while a delegated stale reference remains held elsewhere.

The trusted builtin launch descriptor chooses an executable and explicit
grants together. A common static ELF selects its application model using
descriptive bootstrap metadata. Terminal and Files receive file read/write
and mediated builtin launch; Editor receives file read/write; Settings receives
preferences; Gallery receives neither; Monitor receives a MemoryPool/READ cap.
The file resource scope is the flat `user-*` namespace. Private preferences,
package, permission and trust records are unreachable through that scope.
Configuration commits the versioned application-data record `ui10-prefs` with
real AFS1 complete CoW PUT before changing the running theme/motion state.
The wire describes operations and state, never palette or component styling.

`SYS_OBSERVE=44` is additive and requires a held MemoryPool/READ. It returns nine
bounded counters: free/total frames, spawn records, processes, shared regions,
shared pages, shared mappings, the caller's own occupied cap slots and monotonic
microseconds. It reveals no physical address, grant or lifecycle handle.
MemoryPool/WRITE remains mandatory for allocating SharedRegions. Reserved
arguments and the entire output span are validated before writing.

Six precreated private notifications provide pacing for the six desktop
sessions. Each child receives only its own clock READ|WRITE; it cannot copy it
to others. The parent retains the original reference, drops pending badge state
after consuming the child's Process, and reuses the notification. Existing
timer-owner destruction sweeps armed timers. The notification bound becomes
25, and historical full-fixture tests fill 25 and refuse the 26th without
mutation. There is no per-launch notification allocation.

Native window close first queues an owned close event. Editor can save, discard
or cancel; an owned CancelClose resets the pending decision. A second close
explicitly force-stops an unresponsive child through the broker's held Process.
Normal child exit drives a bounded close reveal before retirement. Open, close
and focus motion use the shared monotonic interpolation system and can be
disabled durably. Window policy and rendering remain entirely userspace.

The first six simultaneous clients consume six reusable intermediate page-table
frames in the broker. SYS_SHARED_UNMAP already removes exact leaf/PTE/span/pin
state while retaining empty intermediate tables until address-space teardown
(ADR-0056). This is bounded resident VM overhead, not an application backing or
per-launch allocation leak. Guest evidence distinguishes cold baseline from
warmed baseline and verifies two further complete six-app cycles plus relaunch
return to identical warmed counters. Six apps use 762 backing pages, plus the
real scanout; the current 8-region/2048-page/32-map registry bounds remain.

The six-app gate proves actual pointer-driven spawn/refusal, ordinary terminal
file operations, file creation, exact editor save bytes, close/cancel/discard,
Settings effects and reboot persistence, monitor pixels, and complete lifecycle
accounting. It does not yet qualify arbitrary dynamic graphical Images,
adversarial function grants, historical regressions or the final exact image.
