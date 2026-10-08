# Phase-14 manual QEMU launch on Windows

The release archive includes `phase14-launch-windows.cmd` and
`phase14-vcon-windows.ps1`. Install a current QEMU for Windows and place the
archive's extracted files together in one directory. Edit `QEMU_EXE` in the
command file if QEMU is installed somewhere other than
`C:\Program Files\qemu\qemu-system-x86_64.exe`.

The command file copies the pristine AFS1/AFS2 template once to
`D:\ArenaOS\phase14-data.img` and copies OVMF's VARS template once to
`phase14-vars.fd` next to the extracted release. Those are separate writable
files; the archived ESP and OVMF CODE remain read-only inputs. Keep the data
disk path distinct from the source archive and from any other ArenaOS disk.
The APB1 source files are already on the guest Desktop. Install the Phase-14
package by double-clicking its APB1 icon, then open All Applications, search
for `Phase14 PIE`, and launch it twice. The serial output shows the signed
installation, relocation result, exit 42, and selected bases.

The command attaches virtio block, QEMU user networking, virtio RNG, virtio
keyboard, virtio tablet, and a virtio-console socket, alongside UEFI/OVMF. A
PowerShell companion connects to the virtio-console socket and answers the
ArenaOS test handshake, so the service-manager bootstrap can finish. The
console listener uses TCP port 4444 on loopback; close any existing listener
on that port before starting.

```bat
phase14-launch-windows.cmd
```

The underlying QEMU invocation is equivalent to:

```bat
qemu-system-x86_64.exe -M q35 -m 512M -cpu qemu64,+nx,+smep,+smap -boot order=c ^
  -drive if=pflash,format=raw,readonly=on,file=edk2-x86_64-code.fd ^
  -drive if=pflash,format=raw,file=phase14-vars.fd ^
  -drive format=raw,file=arena-esp.img ^
  -drive if=none,id=arenaData,format=raw,file=D:\ArenaOS\phase14-data.img ^
  -device virtio-blk-pci,drive=arenaData ^
  -netdev user,id=net0 -device virtio-net-pci,netdev=net0 ^
  -device virtio-rng-pci -device virtio-keyboard-pci -device virtio-tablet-pci ^
  -device virtio-serial-pci,max_ports=1 ^
  -chardev socket,id=console0,host=127.0.0.1,port=4444,server=on,wait=off ^
  -device virtconsole,chardev=console0 -display sdl -serial stdio -no-reboot
```
