@echo off
setlocal
set "ROOT=%~dp0"
if not defined QEMU_EXE set "QEMU_EXE=C:\Program Files\qemu\qemu-system-x86_64.exe"
if not exist "D:\ArenaOS" mkdir "D:\ArenaOS"
if not exist "D:\ArenaOS\phase14-data.img" copy /y "%ROOT%scratch-template.img" "D:\ArenaOS\phase14-data.img" >nul
if not exist "%ROOT%phase14-vars.fd" copy /y "%ROOT%ovmf-vars-template.img" "%ROOT%phase14-vars.fd" >nul

start "ArenaOS virtio-console helper" powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%ROOT%phase14-vcon-windows.ps1" -Port 4444
"%QEMU_EXE%" ^
  -name "ArenaOS Phase 14" -M q35 -m 512M -cpu qemu64,+nx,+smep,+smap -boot order=c ^
  -drive if=pflash,format=raw,readonly=on,file="%ROOT%edk2-x86_64-code.fd" ^
  -drive if=pflash,format=raw,file="%ROOT%phase14-vars.fd" ^
  -drive format=raw,file="%ROOT%arena-esp.img" ^
  -drive if=none,id=arenaData,format=raw,file="D:\ArenaOS\phase14-data.img" ^
  -device virtio-blk-pci,drive=arenaData ^
  -netdev user,id=net0 -device virtio-net-pci,netdev=net0 ^
  -device virtio-rng-pci -device virtio-keyboard-pci -device virtio-tablet-pci ^
  -device virtio-serial-pci,max_ports=1 ^
  -chardev socket,id=console0,host=127.0.0.1,port=4444,server=on,wait=off ^
  -device virtconsole,chardev=console0 -display sdl -serial stdio -no-reboot
if errorlevel 1 exit /b %errorlevel%
