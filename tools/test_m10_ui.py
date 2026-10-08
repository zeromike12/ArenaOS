#!/usr/bin/env python3
"""Host presentation/policy gate. Does not claim running apps or QMP input."""
import hashlib
import subprocess
import tempfile
from pathlib import Path
import arena_env
ROOT=Path(__file__).resolve().parent.parent

def main():
    # Exercise every possible live-stream cut, including a split UTF-8 glyph
    # and a final caps digit. Neither harness may treat a prefix as a sample.
    from test_m10_apps import serial_text, NATIVE_COUNTERS
    from check_phase10_pixels import COUNTERS
    records=[(113704,14,14,1,469,2,16),(112762,20,20,7,1231,14,28)]
    data='boot \u03bb\n'.encode()+b''.join(b'[desktop] measured frames/records/processes/regions/pages/maps/caps='+b'/'.join(str(v).encode() for v in row)+b'\r\n' for row in records)
    with tempfile.TemporaryDirectory() as temporary:
        stream=Path(temporary)/'serial.log'
        for cut in range(len(data)+1):
            prefix=data[:cut];stream.write_bytes(prefix)
            expected=records[:prefix.count(b'\r\n')]
            assert [tuple(map(int,m)) for m in NATIVE_COUNTERS.findall(serial_text(stream))]==expected
            assert [tuple(map(int,m)) for m in COUNTERS.findall(prefix)]==expected
    env=arena_env.rust_env()
    for crate in ('ui','desktop'):
        manifest=str(ROOT/f'userspace/{crate}/Cargo.toml')
        for cmd in (
            ['cargo','fmt','--manifest-path',manifest,'--check'],
            ['cargo','test','--offline','--locked','--manifest-path',manifest,'--lib','--target','x86_64-unknown-linux-gnu'],
            ['cargo','clippy','--offline','--locked','--manifest-path',manifest,'--lib','--target','x86_64-unknown-linux-gnu','--','-D','warnings'],
            ['cargo','build','--offline','--locked','--manifest-path',manifest,'--lib','--release','--target','x86_64-unknown-none'],
        ):subprocess.run(cmd,cwd=ROOT,env=env,check=True)
    subprocess.run(['cargo','clippy','--offline','--locked','--release','--bins',
                    '--target','x86_64-unknown-none','--','-D','warnings'],
                   cwd=ROOT/'userspace/desktop',env=env,check=True)
    for source in ('userspace/inputd/src/pointer.rs','userspace/fsd/src/replace.rs','userspace/desktop/src/apps/model.rs'):
        output=arena_env.build_dir()/(Path(source).stem+'-m10-tests')
        subprocess.run(['rustc','--test','--edition','2024',source,'-o',str(output)],cwd=ROOT,env=env,check=True)
        subprocess.run([str(output)],check=True)
    captures=[]
    for name,args in (('light',[]),('dark',['--dark'])):
        command=['cargo','run','--quiet','--offline','--locked','--manifest-path',str(ROOT/'userspace/ui/Cargo.toml'),
                 '--example','gallery','--target','x86_64-unknown-linux-gnu','--',*args]
        data=subprocess.check_output(command,cwd=ROOT,env=env)
        assert data==subprocess.check_output(command,cwd=ROOT,env=env),'gallery nondeterministic'
        header=b'P6\n448 288\n255\n'; assert data.startswith(header) and len(data)==len(header)+448*288*3
        pixels=data[len(header):]
        # Semantic structure: selected row differs from plain row, focused
        # control has its own border, terminal is distinct from surrounding
        # panel. No specific temporary RGB palette is an invariant.
        # Coordinates follow the Opus gallery layout (sidebar rows at x300,
        # button grid at x12, console sample at y186..256).
        pixel=lambda x,y:pixels[(y*448+x)*3:(y*448+x)*3+3]
        assert pixel(310,86)!=pixel(310,106)
        assert pixel(30,112)!=pixel(30,84)
        assert pixel(140,250)!=pixel(140,260)
        path=arena_env.build_dir()/f'phase10-host-gallery-{name}.ppm'; path.write_bytes(data)
        captures.append(data)
        print(f'host fixture {path.name} sha256={hashlib.sha256(data).hexdigest()}')
    assert captures[0]!=captures[1],'theme has no visible effect'
    print('[m10-ui] host components/motion/window-policy, no_std binaries/static checks and deterministic palette-independent raster checks PASS; guest proof is separate')

if __name__=='__main__':main()
