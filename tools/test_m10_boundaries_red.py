#!/usr/bin/env python3
"""Production boundary mutants; exact restoration and real guest GREEN.

Each control builds actual production code. A build error cannot count RED.
Guest pixel/native/resource oracles must reject the mutant. Source and original
EFI/ESP/profile are restored even on failure; final GREEN boots those exact bytes.
"""
import hashlib
import traceback
from pathlib import Path
import arena_env
import mtest
import test_m10_boundaries as bounded
import test_m10_dynamic as dynamic
import test_m10_files as files
import test_m10_apps as apps
ROOT=arena_env.REPO_ROOT;BUILD=arena_env.build_dir()
MODEL=ROOT/'userspace/desktop/src/model.rs'
DESKTOP=ROOT/'userspace/desktop/src/bin/desktop.rs'
SCOPE=ROOT/'userspace/desktop/src/scope.rs'
KERNEL=ROOT/'kernel/kernel/src/arch/x86_64/syscall.rs'
CONTROLS=[
    (MODEL,b'(self.screen.1 - m::DOCK_HEIGHT - m::TITLE_HEIGHT)',b'(self.screen.1 - m::TITLE_HEIGHT)','dock-safe-title',dynamic,'safe-title-above-dock'),
    (DESKTOP,b'result = u64::from(unsafe { PREFS.dark })',b'result = u64::from(unsafe { PREFS.dark } && s.kind != 0)','live-theme-delivery',apps,'live-theme-terminal'),
    (SCOPE,b'scope & resource != 0 && rights & needed == needed',b'(scope & resource != 0 && rights & needed == needed) || rights != 0','function-scope',bounded,'terminal'),
    (SCOPE,b'kind < 6 && targets & (1 << kind) != 0',b'kind < 6 && (targets & (1 << kind) != 0 || targets != 0)','launch-target-scope',files,'files'),
    # Phase 11.4 routes the pointer through State::target (hit() is host-test
    # only since then): the live window hit test is the boundary.
    (MODEL,b'None if w.contains(x, y) => Some(Target::Window(w.handle)),',b'None if w.contains(x, y) && false => Some(Target::Window(w.handle)),','pointer-hit',bounded,('pointer-focus','terminal-key','exact-drag')),
    (DESKTOP,b'            SYS_SPAWN,\n            image,',b'            SYS_SPAWN_CHECK,\n            image,','real-process-spawn',bounded,'terminal'),
    (DESKTOP,b'if unsafe { syscall2(SYS_PROC_FINISH, s.process, u64::from(force)) } != 0 {',b'if force && unsafe { syscall2(SYS_PROC_FINISH, s.process, u64::from(force)) } != 0 {','process-retirement',bounded,'resources did not return exactly'),
    # Phase 11.6: slot 4 is the diagnostics pool or the /Users/user grant.
    (DESKTOP,b'            (POOL, RIGHTS_READ)\n',b'            (POOL, RIGHTS_READ | RIGHTS_WRITE)\n','readonly-diagnostic-grant',bounded,'monitor'),
    (MODEL,b'self.find(handle).is_some_and(|w| w.backing == backing)',b'self.find(handle).is_some_and(|w| w.backing == backing || backing != 0)','owned-stale-surface',dynamic,'signed-'),
    # Window creation (11.3 transients mint handles the same way).
    (MODEL,b'.ok_or(Error::Full)?;\n        let handle = self.next;\n        let next = self.next.checked_add(1).ok_or(Error::Exhausted)?;',b'.ok_or(Error::Full)?;\n        let handle = self.next;\n        let next = self.next;','surface-generation-reuse',dynamic,'signed-'),
    # Damage composition reads each window's content through one closure;
    # the mutant serves the client's writable staging bytes instead.
    # Phase 11.3: composition reads each session's snapshot via snapshot_of.
    (DESKTOP,b'|slot| snapshot_of(&sessions[slot])',b'|slot| { let s = &sessions[slot]; let w = unsafe { (&*(&raw const WM)).find(s.handle) }.unwrap_or_else(|| die(88)); (unsafe { core::slice::from_raw_parts((s.va as usize + PIXEL_OFFSET) as *const u32, w.width as usize * w.height as usize) }, snapshot_of(s).1) }','frame-publication',dynamic,'unpublished backing became visible'),
    (DESKTOP,b'    if ready != 0 {',b'    if ready != 0 && false {','capacity-preflight',dynamic,'fifth refusal mutated resources'),
]
def main():
    esp=mtest.build('m10-boundaries-red-base',desktop=True)
    artifacts={p:p.read_bytes() for p in (esp,BUILD/'arena-boot.efi',BUILD/'graphics-profile.txt')}
    sources={p:p.read_bytes() for p,_,_,_,_,_ in CONTROLS}
    digest=hashlib.sha256(artifacts[BUILD/'arena-boot.efi']).hexdigest()
    try:
        bounded.main(esp)
        for path,before,after,label,oracle,expectation in CONTROLS:
            original=sources[path];assert original.count(before)==1,(label,original.count(before))
            path.write_bytes(original.replace(before,after,1))
            log=BUILD/f'm10-{label}-red.log'
            try:
                mutant=mtest.build('m10-'+label+'-mutant',desktop=True)
                failed=False
                try:oracle.main(mutant)
                except (AssertionError,RuntimeError) as error:
                    observation=''.join(traceback.format_exception(error));log.write_text(observation)
                    failed=any(e in observation for e in (expectation if isinstance(expectation,tuple) else (expectation,)))
                assert failed,f'{label}: real guest oracle did not reject expected production boundary'
                print(f'[m10-boundaries-red] production {label}: actual guest oracle RED PASS',flush=True)
            finally:path.write_bytes(original)
    finally:
        for p,data in sources.items():p.write_bytes(data)
        try:mtest.build('m10-boundaries-exact-restored',desktop=True)
        finally:
            for p,data in artifacts.items():p.write_bytes(data)
    assert all(p.read_bytes()==data for p,data in sources.items())
    assert all(p.read_bytes()==data for p,data in artifacts.items())
    try:
        bounded.main(esp)
        dynamic.main(esp)
    finally:
        for p,data in artifacts.items():p.write_bytes(data)
    assert all(p.read_bytes()==data for p,data in sources.items())
    assert all(p.read_bytes()==data for p,data in artifacts.items())
    print(f'[m10-boundaries-red] {len(CONTROLS)} real production RED controls; byte-exact source/EFI/ESP restored, native/pointer/process and signed dynamic graphical GREEN sha256={digest}',flush=True)
if __name__=='__main__':main()
