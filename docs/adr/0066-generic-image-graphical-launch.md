# ADR-0066: Generic graphical launch by held executable capability

Status: implemented; signed dynamic guest workflow green; mutation and full qualification pending.

The desktop endpoint accepts a canonical LaunchImage request carrying an actual
live Image/READ capability. Possession of that executable is the execution
authority; no package name, PID, surface handle or caller-supplied application
role supplies it. The broker creates a fresh backing, retains the exact
Process/READ|DESTROY returned by SYS_SPAWN, and installs the session association
before serving the child. The incoming Image reference is disposed after
spawn. Loader pins/copied pages and registrar revocation remain unchanged.

An arbitrary Image receives only the caller endpoint, own graphical region,
an attenuated READ|COPY function reference with **no provisioned function
scope**, and a private pacing clock. No raw fsd/display/input, configuration,
package, pool or diagnostic authority is inherited. Builtin descriptors remain
separate trusted launch policy; a dynamic executable cannot request a builtin
role to gain its function scope. Startup metadata for unknown external models
is refused rather than pretending they are the builtin Gallery.

`SYS_SPAWN_CHECK=45` is an additive, read-only Image/READ or BootImage/READ
preflight. It validates full executable liveness before reporting bounded
dynamic-child, spawn-record, process and caller-cap capacity. It reserves
nothing and confers no new authority. SYS_SPAWN still repeats validation and
its rollback remains authoritative. The broker performs preflight before
auxiliary SharedRegion allocation. In particular, a fifth dynamic child
refuses before consuming a new region ID or retaining an empty auxiliary
mapping page table. A conflict after observation still rolls back resources.

Production boot now selects Desktop explicitly, including keyboard-only
configurations. `ARENA_GRAPHICS_FIXTURE=phase9` selects the preserved Phase-9
root provisioning for historical graphics tests. `mtest.build` selects this
named fixture by default; Phase-10 desktop gates request `desktop=True`.
`tools/build.sh` without the fixture variable builds the shipping desktop and
records its profile beside the EFI. Device count no longer selects policy.

Boot root explicitly grants the manager a late desktop Endpoint/WRITE|COPY in
slot 31. It does not grant an executable or new package authority there. Slot
25 was experimentally occupied by an in-progress boot worker; using it caused
the correct kernel refusal and fail-stop. Slot 31 is reserved for this late
grant and is absent from the legacy fixture. The manager's trusted serial
diagnostic `pkg graphics` uses the existing signed QUERY/INSTALL/PREPARE/COMMIT
transaction, receives a real dynamic Image, and sends that held cap to the
broker. `pkg graphicsrevoke` exercises existing registrar revocation while
copied child code is live. These opt-in diagnostics are not ambient application
package permissions or authorization through an application name.

`tools/test_m10_dynamic.py` signs a real 1384-byte static ET_EXEC (inside the
unchanged 4096-byte payload ceiling), validates it with the production ELF
validator, stages public signed bytes on AFS1, and executes four instances via
the real package receiver/manager/broker. Each creates and paints its own
surface, refuses a forged/previous handle and an ungranted configuration call.
QMP checks owned-raster keyboard change, exact drag translation and close.
Fifth-child refusal returns identical actual resource counters while two
desktop session slots remain available. Full-ID revoke refuses another spawn
while input continues changing a surviving child's copied-code raster.
All four original Process owners are consumed on close; disk records remain
independently audited. A cursor-only change cannot satisfy the input proof.

The ELF fixture initially linked generic slice panic formatting and exceeded
the package ceiling. Fixed in-bounds wire stores removed that unused formatting
dependency; no payload format, signature policy, dynamic linking or image
budget was enlarged. Arbitrary large packaged desktop applications remain
limited by the accepted Phase-8.5 format until a separately reviewed phase.
