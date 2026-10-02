#!/usr/bin/env bash
# Boot-stability loop (docs/TESTING.md): boot the *prebuilt* ESP image N
# times with fresh NVRAM each run and require the full green verdict on
# every single boot — every mN RESULT line, the TCP guest/host wire
# proof, the canonical clean-halt line, no PANIC, QEMU exit 0, under a per-boot
# timeout. Every fixture rides along (the AFS1 scratch disk, the slirp
# NIC, the entropy source, the virtio keyboard, and — since M6.4 — the
# virtio-console port) —
# stability means the SHIPPING configuration. Boots therefore use BOTH
# input channels: `shutdown` typed on the serial chardev, and the m6
# input fixture typed on the KEYBOARD through QMP (tools/qmp.py), each
# paced by its own serial marker.
#
# A single green boot proves correctness; a hundred prove the kernel is
# not winning a race (fresh-vars nondeterminism, TCG timing jitter).
#
# Usage: tools/stability_loop.sh [N]        (default N=100)
#        tools/build.sh --image             (must run first)
set -euo pipefail

N="${1:-100}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -f "$REPO_ROOT/tools/dev-env/env.sh" ]]; then
    # shellcheck disable=SC1091
    source "$REPO_ROOT/tools/dev-env/env.sh"
fi

ESP="$REPO_ROOT/build/arena-esp.img"
if [[ ! -f "$ESP" ]]; then
    echo "error: $ESP missing — run tools/build.sh --image first" >&2
    exit 1
fi

# Resolve QEMU/OVMF exactly like the test harness does.
eval "$(cd "$REPO_ROOT" && python3 - "$REPO_ROOT/build/vcon-stability.sock" <<'EOF'
import sys
from pathlib import Path
sys.path.insert(0, str(Path("tools").resolve()))
import arena_env
print(f"QEMU=({' '.join(repr(x) for x in arena_env.qemu_cmd() + arena_env.qemu_data_args())})")
print(f"OVMF_CODE={arena_env.ovmf_code()}")
print(f"OVMF_VARS={arena_env.ovmf_vars_template()}")
print(f"SCRATCH=({' '.join(repr(x) for x in arena_env.scratch_disk_args())})")
print(f"NET=({' '.join(repr(x) for x in arena_env.net_args())})")
print(f"RNG=({' '.join(repr(x) for x in arena_env.rng_args())})")
print(f"KBD=({' '.join(repr(x) for x in arena_env.input_args())})")
print(f"VCON=({' '.join(repr(x) for x in arena_env.console_args(Path(sys.argv[1])))})")
EOF
)"

VARS="$REPO_ROOT/build/ovmf-vars-stability.img"
SERIAL="$REPO_ROOT/build/stability-serial.log"
QMP_SOCK="$REPO_ROOT/build/qmp-stability.sock"
VCON_SOCK="$REPO_ROOT/build/vcon-stability.sock"
BOOT_TIMEOUT=60          # healthy TCG boot is <10s; hang = failure
# The suite verdicts are DERIVED, not written down here.
#
# This file used to carry a hardcoded line per suite — m2 21/21, m3
# 13/13, … m6 4/4 — and every milestone that added a test had to
# remember to come back and edit it. M6.4 and M6.5 added two, nobody
# edited it, and the v0.10.0 tag shipped a qualification tool that
# scores 0/100 on its own image: it was still looking for
# 'm6: RESULT PASS (4/4)'. The check was wrong in the one direction a
# test must never be wrong — it failed a machine that was fine, and
# would equally have passed a machine that had silently LOST tests,
# because 4/4 is a perfectly good line for a suite that used to have
# six.
#
# So: boot 1 establishes the expectation, and boots 2..N must match it
# exactly. Every 'mN: RESULT ...' line the kernel prints is captured
# as a set; a suite that changes its verdict, loses tests, or stops
# reporting at all is a difference. Nothing to keep in step by hand,
# and the expectation is printed so it is never a mystery.
EXPECTED="$REPO_ROOT/build/stability-expected.txt"
# The keystrokes the m6 input_service test waits for, typed on the
# virtual keyboard once inputd announces DRIVER_OK.
KEY_MARKER='inputd: virtio-input ready'
KEY_TEXT='arena'
# The console port's far end: contest sends this line out the port and
# waits for the answer, so a boot with the device attached needs
# somebody at the socket (M6.4, ADR-0027).
VCON_MARKER='contest: hello from ArenaOS'
VCON_REPLY='host-says-hello
'
HALT_LINE='halting via UEFI ResetSystem(shutdown)'

pass=0
fail=0
t_start=$(date +%s)
# Derived FRESH each run: the expectation is "every boot of THIS image
# agrees", not "this image matches whatever ran here last week". A
# stale file would reintroduce exactly the staleness this replaced.
rm -f "$EXPECTED" "$REPO_ROOT/build/stability-receipt.txt"
# 0.2 s polls per second of boot timeout: the feeder must never outlive
# QEMU's timeout, or a boot that dies before the prompt wedges the
# pipeline forever (timeout kills QEMU, not the grep loop).
FEED_ITERS=$((BOOT_TIMEOUT * 5))

for i in $(seq 1 "$N"); do
    cp "$OVMF_VARS" "$VARS"              # fresh NVRAM every boot
    # Fresh AFS1 scratch disk EVERY boot (the fixture contract, and
    # load-bearing since M5.3: fs_service's fstest CREATEs a fixed
    # filename — a disk inherited from the previous boot answers
    # FS_ERR_EXISTS and fails the suite by design).
    ( cd "$REPO_ROOT" && python3 -c 'import sys; sys.path.insert(0, "tools"); import arena_env; arena_env.make_scratch_disk()' >/dev/null )
    rm -f "$SERIAL" "$QMP_SOCK" "$VCON_SOCK" "$REPO_ROOT/build/tcp-stability.log" "$REPO_ROOT/build/udp-dns-stability.log"
    # M7.6: bind the host TCP fixture BEFORE QEMU starts. READY is
    # emitted only after listen() succeeds; no sleep/race and no
    # in-guest fake peer. One actor, one boot, like the typist.
    coproc TCP_PEER { exec python3 -u "$REPO_ROOT/tools/network_fixture.py" \
        "$REPO_ROOT/build/tcp-stability.log" \
        "$REPO_ROOT/build/udp-dns-stability.log"; }
    tcp_pid=$TCP_PEER_PID
    if ! read -r ready <&"${TCP_PEER[0]}" || [[ "$ready" != READY ]]; then
        echo "boot $i: FAIL — TCP/UDP host actor could not bind ports 54321/1053" >&2
        exit 1
    fi
    rc=0
    # The keyboard typist: waits for inputd's ready marker on the
    # serial log, then types the input fixture through QMP. It is
    # REAPED after this boot (below) rather than left to expire: the
    # serial log is recreated every iteration, so a typist that
    # outlived its own boot would find the NEXT boot's marker and type
    # into it — a stray `arena` landing in the same line as the
    # feeder's `shutdown` would hang a perfectly good kernel. One
    # typist, one boot.
    python3 "$REPO_ROOT/tools/qmp.py" "$QMP_SOCK" "$SERIAL" \
        "$KEY_MARKER" "$KEY_TEXT" "$BOOT_TIMEOUT" >/dev/null 2>&1 &
    typist_pid=$!
    # Phase-9 per-boot independent before/after QMP captures. The actor
    # waits for both real windows, injects a virtio key, verifies the
    # focused client's newly painted pixel, and checks preserved base.
    # The serial feeder waits for its receipt before running stacktest.
    PIXEL_RECEIPT="$REPO_ROOT/build/stability-pixels-boot-$i.txt"
    PIXEL_IMAGE="$REPO_ROOT/build/stability-display-boot-$i.ppm"
    PIXEL_ERROR="$PIXEL_RECEIPT.error"
    rm -f "$PIXEL_RECEIPT" "$PIXEL_IMAGE" "$PIXEL_ERROR"
    python3 "$REPO_ROOT/tools/check_phase9_pixels.py" "$QMP_SOCK" "$SERIAL" \
        "$PIXEL_IMAGE" "$PIXEL_RECEIPT" "$BOOT_TIMEOUT" \
        >"$REPO_ROOT/build/stability-pixel-actor.log" 2>&1 &
    pixel_pid=$!
    # The console actor, reaped with the typist for the same reason.
    python3 "$REPO_ROOT/tools/vcon.py" "$VCON_SOCK" \
        "$VCON_MARKER" "$VCON_REPLY" "$BOOT_TIMEOUT" \
        "$REPO_ROOT/build/vcon-port.txt" >/dev/null 2>"$REPO_ROOT/build/vcon-dbg.txt" &
    vcon_pid=$!
    # ADR-0040/0041: qualify one real manager restart on EACH boot;
    # the dedicated stress QEMU fixture proves all three and budget. Wait for both prompt and first manager ready, explicitly
    # type the privileged stacktest, then wait for its end-to-end
    # success AND second shell prompt before shutdown. A failure or
    # hang times out — it cannot be retried into an apparent pass.
    {
        n=0
        while ! grep -aq 'arena>' "$SERIAL" 2>/dev/null || \
              ! grep -aqF 'servicemgr: production netstackd READY pid' "$SERIAL" 2>/dev/null; do
            sleep 0.2; n=$((n + 1)); if (( n >= FEED_ITERS )); then exit 0; fi
        done
        # Keyboard 'q' was injected by the pixel actor. It is ALSO a
        # console byte, so erase it in the shared shell line discipline
        # before issuing stacktest; do not race the input image receipt.
        n=0
        while [[ ! -s "$PIXEL_RECEIPT" && ! -s "$PIXEL_ERROR" ]]; do
            sleep 0.2; n=$((n + 1)); if (( n >= FEED_ITERS )); then exit 0; fi
        done
        printf '\bstacktest\r'
        n=0
        while ! grep -aqF 'm8: stacktest PASS (same endpoint; old bearer revoked; fresh ARP request on real wire)' "$SERIAL" 2>/dev/null || \
              (( $(grep -ac 'arena>' "$SERIAL" 2>/dev/null || true) < 2 )); do
            sleep 0.2; n=$((n + 1)); if (( n >= FEED_ITERS )); then exit 0; fi
        done
        printf 'shutdown\r'
        n=0
        while ! grep -aqF "$HALT_LINE" "$SERIAL" 2>/dev/null; do
            sleep 0.2; n=$((n + 1)); if (( n >= FEED_ITERS )); then exit 0; fi
        done
    } | timeout "$BOOT_TIMEOUT" "${QEMU[@]}" \
        -M q35 -m 512M -cpu qemu64,+nx,+smep,+smap \
        -boot order=c \
        -drive if=pflash,format=raw,readonly=on,file="$OVMF_CODE" \
        -drive if=pflash,format=raw,file="$VARS" \
        -drive format=raw,file="$ESP" \
        "${SCRATCH[@]}" \
        "${NET[@]}" \
        "${RNG[@]}" \
        "${KBD[@]}" \
        "${VCON[@]}" \
        -qmp unix:"$QMP_SOCK",server=on,wait=off \
        -display none -chardev stdio,id=con0,signal=off -serial chardev:con0 \
        -no-reboot > "$SERIAL" 2>/dev/null || rc=$?

    # Reap this boot's typist before the next one starts.
    kill "$typist_pid" 2>/dev/null || true
    wait "$typist_pid" 2>/dev/null || true
    pixel_actor_rc=0
    wait "$pixel_pid" 2>/dev/null || pixel_actor_rc=$?
    # Hash receipts retain evidence for every boot; preserve two actual
    # PPMs as viewable samples without accumulating 100 full frames.
    if (( i != 1 && i != N )) && (( pixel_actor_rc == 0 )); then
        rm -f "$PIXEL_IMAGE"
    fi
    kill "$vcon_pid" 2>/dev/null || true
    wait "$vcon_pid" 2>/dev/null || true
    # The peer exits when it sees the guest's orderly FIN. Reap it so
    # its listener cannot trespass into the next boot's fixture.
    kill "$tcp_pid" 2>/dev/null || true
    wait "$tcp_pid" 2>/dev/null || true
    cp "$REPO_ROOT/build/udp-dns-stability.log" \
        "$REPO_ROOT/build/udp-dns-stability-boot-$i.log" 2>/dev/null || true

    why=""
    if (( rc != 0 )); then
        why="qemu exit rc=$rc (timeout is 124)"
    elif (( pixel_actor_rc != 0 )) || [[ ! -s "$PIXEL_RECEIPT" ]]; then
        why="QMP graphical pixels were not independently verified ($(cat "$PIXEL_ERROR" 2>/dev/null || cat "$REPO_ROOT/build/stability-pixel-actor.log" 2>/dev/null || true))"
    elif ! grep -Eq '^800x600 [0-9a-f]{64}$' "$PIXEL_RECEIPT" || \
         ! grep -Eq '^INPUT [0-9a-f]{64} [0-9a-f]{64}$' "$PIXEL_RECEIPT"; then
        why="QMP owned compositor / injected-key pixel receipt malformed"
    elif [[ ! -f "$SERIAL" ]]; then
        why="no serial output"
    elif grep -aq 'PANIC' "$SERIAL"; then
        why="kernel PANIC on serial"
    elif grep -aqE '\[arena ERROR (halt|ipc)\]|halting machine:' "$SERIAL"; then
        why="kernel halt/IPC error on serial (ResetSystem exit 0 is NOT success)"
    elif ! grep -aq 'TCP_FIXTURE_PASS request=arena-tcp bytes=200 eof=True' \
        "$REPO_ROOT/build/tcp-stability.log"; then
        why="TCP host peer did not verify request/200-byte response/FIN"
    elif [[ $(grep -c 'DNS_FIXTURE_QUERY ' "$REPO_ROOT/build/udp-dns-stability.log" || true) -ne 3 ]]; then
        why="host UDP fixture did not observe exactly three independent DNS requests"
    elif grep -aq 'TCP SKIP' "$SERIAL"; then
        why="TCP was skipped although this is a fixture-equipped qualification"
    elif [[ $(grep -acF 'm83: returncap PASS (40 real reply caps rejected and discarded; slot 2 empty, occupancy 2/32 exact; ordinary no-cap PING unchanged)' "$SERIAL" || true) -ne 2 ]]; then
        why="linked IPC client did not discard forty landed caps in BOTH real M6 server/client fixtures"
    elif ! grep -aq 'native UDP API bound, sent, drained the real multi-IPC response, and revoked its bearer' "$SERIAL"; then
        why="native UDP library did not deliver/revoke its real-wire datagram"
    elif ! grep -aq 'TCP FIN was acknowledged, the peer closed, and the bearer was revoked' "$SERIAL"; then
        why="guest did not complete TCP FIN/close/revocation"
    # Shipping fixture: real manager caps, two bounded active driver
    # probes, then two production children and a real-wire restart.
    # Destructive fault/stall negatives live in the historical host suite.
    elif ! grep -aqF 'audited 22 literal caps; no device/Power/Process grants' "$SERIAL"; then
        why="manager bootstrap cap audit absent on full fixture"
    elif ! grep -aqF 'servicemgr: full fixture notification budget 18/18; nineteenth refused' "$SERIAL"; then
        why="full fixture notification bound was not tested"
    elif ! grep -aqF 'servicemgr: policy validated from live caps and ready drivers' "$SERIAL"; then
        why="ring-3 manager did not validate live inventory and driver readiness"
    elif [[ $(grep -acF 'servicemgr: active netd MAC and rngd entropy probes passed; worker reaped' "$SERIAL" || true) -ne 2 ]]; then
        why="two actual pre-spawn driver probes/reaps were not observed"
    elif [[ $(grep -acF 'depcheck: rngd device completed 64 varied bytes' "$SERIAL" || true) -ne 2 ]]; then
        why="the entropy device did not complete both active dependency probes"
    elif [[ $(grep -acF 'depcheck: production driver poison opcodes refused without marker' "$SERIAL" || true) -ne 2 ]]; then
        why="ordinary worker endpoints did not reject driver poison on both probe rounds"
    elif ! grep -aqF 'servicemgr: production netstackd READY pid' "$SERIAL"; then
        why="manager did not bring its production child to readiness"
    elif ! grep -aqF 'five inherited child caps audited (netd/W stack/R backoff/RW rngd/W diag/R), IPC landings excluded' "$SERIAL"; then
        why="manager-owned child kernel cap audit missing"
    elif [[ $(grep -acF 'servicemgr: production netstackd READY pid' "$SERIAL" || true) -ne 2 ]]; then
        why="manager did not start exactly two child incarnations"
    elif [[ $(grep -acF 'five inherited child caps audited (netd/W stack/R backoff/RW rngd/W diag/R), IPC landings excluded' "$SERIAL" || true) -ne 2 ]]; then
        why="both manager children were not independently audited"
    elif ! grep -aqF 'servicemgr: production child reaped through Process cap; bounded backoff' "$SERIAL"; then
        why="production child was not reaped/backed off by manager"
    elif ! grep -aqF 'm8: stacktest Process-cap stop refused to endpoint-only client' "$SERIAL"; then
        why="the client endpoint did not prove it lacks Process-stop authority"
    elif ! grep -aqF 'm8: stacktest service refused missing/wrong shutdown marker' "$SERIAL"; then
        why="service-side orderly-stop authority refusals absent"
    elif ! grep -aqF 'm8: stacktest observed SERVICE_GONE during child absence' "$SERIAL"; then
        why="old endpoint did not report SERVICE_GONE in the dead interval"
    elif ! grep -aqF 'm8: stacktest PASS (same endpoint; old bearer revoked; fresh ARP request on real wire)' "$SERIAL"; then
        why="post-restart stale bearer and fresh wire proof missing"
    elif grep -aq 'servicemgr: OFFLINE' "$SERIAL"; then
        why="manager went OFFLINE although both fixture drivers were attached"
    # ADR-0046 partial 8.1 boundary: the data endpoint alone cannot
    # mutate; 20 missing/wrong-kind calls were rejected at the receiver
    # and the guest still scanned the real FS twice without a write.
    elif ! grep -aqF 'configd spawned: pid' "$SERIAL"; then
        why="resident configd did not receive its literal boot grants"
    elif ! grep -aqF 'configread: SET absent/wrong-kind refused x20 by receiver' "$SERIAL"; then
        why="ordinary config endpoint bypassed service-side mutation refusal"
    elif ! grep -aqF 'configread: READ UNSET' "$SERIAL"; then
        why="config reader did not scan the fresh real AFS1 volume"
    elif ! grep -aqF 'configread: ORDINARY READ BOUNDARY PASS (no fsd or marker grant)' "$SERIAL"; then
        why="ordinary config reader did not survive/verify its data-plane proof"
    elif ! grep -aqF 'configread: boot-root reader reaped; no update authority delegated' "$SERIAL"; then
        why="config reader was not reaped before shell startup"
    elif ! grep -aqF 'configup: SKIP (no trusted test intent; no SET)' "$SERIAL"; then
        why="fresh ordinary boot incorrectly ran an authorized configuration update"
    elif ! grep -aqF 'configup: boot-root updater reaped; marker never delegated to shell' "$SERIAL"; then
        why="config updater or its marker survived shell handoff"
    elif grep -aqF 'configup: SET COMMITTED' "$SERIAL"; then
        why="fresh boot unexpectedly consumed an immutable generation"
    # 8.2 durable completion: fresh formatted boot must recover UNSET as
    # DENY. Worker success plus exit and real child-cap audits are mandatory
    # on every qualified boot. Crash/restart negatives live in host suites.
    elif ! grep -aqF 'permissiond: validated durable policy generation 0 DENY' "$SERIAL"; then
        why="fresh permission disk did not rehydrate default DENY"
    elif ! grep -aqF 'permissiond READY (validated durable decision; bearer table fresh)' "$SERIAL"; then
        why="permission broker did not pass authenticated PING/readiness"
    elif ! grep -aqF 'permissiond: audited four inherited caps FS/W RNG/W mediator/R marker/R; 28 extras empty' "$SERIAL"; then
        why="real broker grants differed from exact four-cap inventory"
    elif ! grep -aqF 'permapp: audited ONLY mediator WRITE|COPY, 31 other cap slots empty' "$SERIAL"; then
        why="client app was not independently audited as endpoint-only"
    elif ! grep -aqF 'servicemgr: permission app reaped through held Process cap' "$SERIAL"; then
        why="independent app still live or its manager Process cap not reaped"
    elif ! grep -aqF 'servicemgr: permission PING result + exit before deadline; worker reaped' "$SERIAL"; then
        why="permission readiness lacked a result-and-exit witness"
    elif grep -aqF 'servicemgr: permission PING failed/deadline; no READY' "$SERIAL"; then
        why="permission readiness failed during ordinary qualification"
    # 8.5: even an empty staging namespace is scanned under all five
    # exact attenuated packaged grants. The manager demands a real PING
    # result AND worker exit before announcing READY.
    elif ! grep -aqF 'packaged: boot with exact FS/W endpoint/R STAGE/R registrar/W lifecycle/R; namespace scan verified' "$SERIAL"; then
        why="package receiver did not scan fresh AFS1 under exact grants"
    elif ! grep -aqF 'servicemgr: packaged READY (full boot scan; exact PING + exit + deadline)' "$SERIAL"; then
        why="package receiver did not pass result-and-exit readiness"
    elif ! grep -aqF '[sharedprobe] capacity/rights/zero PASS' "$SERIAL" || \
         ! grep -aqF 'dead-process mapping/cap sweep frame-exact; RESULT PASS (1/1)' "$SERIAL"; then
        why="bounded SharedRegion guest capacity/authority/teardown proof absent"
    elif ! grep -aqF '[displayd] SharedRegion guest authority/zero/copy/mapping PASS' "$SERIAL" || \
         ! grep -aqF '[displayd] ring3 GOP pixels ready (staged font v2)' "$SERIAL"; then
        why="isolated display service did not paint the verified QMP pixels"
    elif grep -aq 'RESULT FAIL' "$SERIAL"; then
        why="a suite reported RESULT FAIL"
    elif ! grep -aqF "$HALT_LINE" "$SERIAL"; then
        why="missing clean-halt declaration"
    fi

    # The derived verdict set: every suite's RESULT line, in order.
    if [[ -z "$why" ]]; then
        verdicts="$(grep -ao '^m[0-9]*: RESULT [A-Z]* ([0-9]*/[0-9]*' "$SERIAL" \
                    | sed 's/\r//' | sort -u)"
        if [[ -z "$verdicts" ]]; then
            why="no suite reported a RESULT line at all"
        elif [[ ! -f "$EXPECTED" ]]; then
            # Boot 1 sets the bar, and says so out loud.
            printf '%s\n' "$verdicts" > "$EXPECTED"
            echo "expectation derived from boot $i:"
            sed 's/^/    /' "$EXPECTED"
            # Every suite must have passed everything it ran; an
            # honest SKIP is allowed (an absent fixture), a partial
            # pass is not.
            if printf '%s\n' "$verdicts" | grep -q 'RESULT PASS' ; then
                while read -r line; do
                    case "$line" in
                        *"RESULT PASS"*)
                            got="${line##*(}"
                            if [[ "${got%%/*}" != "${got##*/}" ]]; then
                                why="boot $i: '$line' passed only part of its suite"
                            fi
                            ;;
                    esac
                done <<< "$verdicts"
            else
                why="boot $i reported no PASSing suite"
            fi
        elif ! printf '%s\n' "$verdicts" | diff -q - "$EXPECTED" >/dev/null; then
            why="the suite verdicts differ from boot 1's ($(printf '%s' "$verdicts" | tr '\n' ';'))"
        fi
    fi

    if [[ -z "$why" ]]; then
        pass=$((pass + 1))
    else
        fail=$((fail + 1))
        saved="$REPO_ROOT/build/stability-fail-$i.log"
        cp "$SERIAL" "$saved" 2>/dev/null || true
        # Keep the console port's own evidence beside the serial log:
        # what the host actor received, and what it did about it.
        cp "$REPO_ROOT/build/vcon-port.txt" \
           "$REPO_ROOT/build/stability-fail-$i-port.txt" 2>/dev/null || true
        cp "$REPO_ROOT/build/vcon-dbg.txt" \
           "$REPO_ROOT/build/stability-fail-$i-actor.txt" 2>/dev/null || true
        echo "boot $i: FAIL — $why (serial saved to ${saved#"$REPO_ROOT"/})"
    fi

    if (( i % 10 == 0 )); then
        echo "progress: $i/$N boots (pass=$pass fail=$fail, $(( $(date +%s) - t_start ))s)"
    fi
done

dt=$(( $(date +%s) - t_start ))
echo "=============================================="
echo "stability: $pass/$N boots fully green in ${dt}s (fail=$fail)"
if (( fail > 0 )); then
    echo "STABILITY: FAIL"
    exit 1
fi
# The receipt: WHICH image was qualified, and how thoroughly. A
# release refuses to publish unless a receipt exists for the exact ESP
# it just built — "100/100" in release notes should be a checkable
# fact about that artifact, not a memory of a run against some earlier
# build (v0.10.0 shipped a stability tool that could not have scored
# 100/100 on its own image, and nothing noticed).
# Anchored on the KERNEL image, not the ESP: the ESP is a FAT volume
# and its bytes change every time it is rebuilt (directory
# timestamps), so hashing it would make the receipt useless within a
# second of being written. arena-boot.efi is what actually determines
# how the machine behaves, and it only changes when the kernel is
# genuinely relinked.
printf '%s %s/%s\n' \
    "$(sha256sum "$REPO_ROOT/build/arena-boot.efi" | cut -d' ' -f1)" "$pass" "$N" \
    > "$REPO_ROOT/build/stability-receipt.txt"
echo "receipt: build/stability-receipt.txt ($(cat "$REPO_ROOT/build/stability-receipt.txt"))"
echo "STABILITY: PASS"
