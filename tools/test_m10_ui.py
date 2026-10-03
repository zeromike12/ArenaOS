#!/usr/bin/env python3
"""Host presentation/policy gate. Does not claim running apps or QMP input."""
import hashlib
import subprocess
from pathlib import Path
import arena_env
ROOT=Path(__file__).resolve().parent.parent

def main():
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
        pixel=lambda x,y:pixels[(y*448+x)*3:(y*448+x)*3+3]
        assert pixel(119,132)!=pixel(119,160)
        assert pixel(118,58)!=pixel(224,58)
        assert pixel(332,132)!=pixel(322,132)
        path=arena_env.build_dir()/f'phase10-host-gallery-{name}.ppm'; path.write_bytes(data)
        captures.append(data)
        print(f'host fixture {path.name} sha256={hashlib.sha256(data).hexdigest()}')
    assert captures[0]!=captures[1],'theme has no visible effect'
    print('[m10-ui] host components/motion/window-policy, no_std binaries/static checks and deterministic palette-independent raster checks PASS; guest proof is separate')

if __name__=='__main__':main()
