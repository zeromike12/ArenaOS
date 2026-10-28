# DIAGNOSTIC DESKTOP PREVIEW -- XF NOT RUN. Windows PowerShell 5.1, no Python.
$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot
$qemu = if ($env:QEMU_EXE) { $env:QEMU_EXE } else { 'C:\Program Files\qemu\qemu-system-x86_64.exe' }
if (-not (Test-Path -LiteralPath $qemu -PathType Leaf)) { throw "QEMU not found at $qemu. Set QEMU_EXE to the full path of qemu-system-x86_64.exe." }
if ($root.Contains(',')) { throw 'Extract the preview to a path without commas (QEMU drive options use commas).' }
$work = Join-Path $root 'run'
New-Item -ItemType Directory -Force -Path $work | Out-Null
$data = Join-Path $work 'preview-data.img'
$vars = Join-Path $work 'preview-vars.fd'
if (-not (Test-Path -LiteralPath $data)) { Copy-Item -LiteralPath (Join-Path $root 'preview-data-template.img') -Destination $data }
if (-not (Test-Path -LiteralPath $vars)) { Copy-Item -LiteralPath (Join-Path $root 'ovmf-vars-template.fd') -Destination $vars }
$serial = Join-Path $work 'serial.log'
Remove-Item -LiteralPath $serial -ErrorAction SilentlyContinue
# Each job binds before QEMU starts. Only loopback is exposed. They implement
# the same boot-test protocols as tools/tcp_fixture.py and udp_dns_fixture.py.
$token = [Guid]::NewGuid().ToString('N')
$tcpReady = Join-Path $work "tcp-$token.ready"
$udpReady = Join-Path $work "udp-$token.ready"
$tcp = Start-Job -ArgumentList $tcpReady -ScriptBlock {
    param($ready)
    $ErrorActionPreference = 'Stop'
    $l = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback,54321)
    try {
        $l.Start(); [IO.File]::WriteAllText($ready, 'READY')
        $c = $l.AcceptTcpClient(); $c.ReceiveTimeout = 20000
        try {
            $s = $c.GetStream(); $buf = [byte[]]::new(9); $n = 0
            while ($n -lt 9) { $k = $s.Read($buf,$n,9-$n); if ($k -eq 0) { break }; $n += $k }
            if ($n -ne 9 -or [Text.Encoding]::ASCII.GetString($buf) -ne 'arena-tcp') { throw 'Unexpected TCP fixture request' }
            $base = [Text.Encoding]::ASCII.GetBytes('arenaos!'); $out = [byte[]]::new(200)
            for ($i=0; $i -lt 200; $i++) { $out[$i] = $base[$i % 8] -bxor $i }
            $s.Write($out,0,$out.Length); $c.Client.Shutdown([System.Net.Sockets.SocketShutdown]::Send)
            [void]$s.ReadByte()
        } finally { $c.Dispose() }
    } finally { $l.Stop() }
}
$udp = Start-Job -ArgumentList $udpReady -ScriptBlock {
    param($ready)
    $ErrorActionPreference = 'Stop'
    $u = [System.Net.Sockets.UdpClient]::new([System.Net.IPEndPoint]::new([System.Net.IPAddress]::Loopback,1053))
    try {
        [IO.File]::WriteAllText($ready, 'READY')
        $question = [byte[]](7,101,120,97,109,112,108,101,3,99,111,109,0,0,1,0,1)
        $expected = [byte[]](1,0,0,1,0,0,0,0,0,0) + $question
        $suffix = [byte[]](0xc0,12,0,1,0,1,0,0,0,60,0,4,93,184,215,14,0xc0,12,0,1,0,1,0,0,0,60,0,4,93,184,215,15)
        while ($true) {
            $remote = [System.Net.IPEndPoint]::new([System.Net.IPAddress]::Any,0)
            $p = $u.Receive([ref]$remote)
            if ($p.Length -ne 29) { continue }
            $valid = $true
            for ($i=0; $i -lt $expected.Length; $i++) { if ($p[$i+2] -ne $expected[$i]) { $valid = $false; break } }
            if (-not $valid) { continue }
            $reply = [byte[]](@($p[0],$p[1],0x81,0x80,0,1,0,2,0,0,0,0) + $question + $suffix)
            [void]$u.Send($reply,$reply.Length,$remote)
        }
    } finally { $u.Dispose() }
}
try {
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    while (-not ((Test-Path -LiteralPath $tcpReady) -and (Test-Path -LiteralPath $udpReady))) {
        if ($tcp.State -in @('Failed','Stopped','Completed') -or $udp.State -in @('Failed','Stopped','Completed')) { Receive-Job $tcp,$udp; throw 'A local boot peer failed to start (ports 54321 and 1053 must be free).' }
        if ([DateTime]::UtcNow -ge $deadline) { throw 'Timed out starting local boot peers.' }
        Start-Sleep -Milliseconds 100
    }
    Write-Host 'DIAGNOSTIC DESKTOP PREVIEW -- XF NOT RUN (not strict m15 PASS)'
    Write-Host 'When serial says inputtest: input-service client starting, click the QEMU window and type arena (no Enter).'
    Write-Host 'When finished, type shutdown and Enter in this serial console for a clean stop.'
    $argsQemu = @('-name','ArenaOS Phase 16 DIAGNOSTIC DESKTOP PREVIEW - XF NOT RUN', '-accel','tcg', '-M','q35', '-m','512M', '-cpu','qemu64,+nx,+smep,+smap',
      '-drive',("if=pflash,format=raw,readonly=on,file=" + (Join-Path $root 'ovmf-code.fd')),
      '-drive',("if=pflash,format=raw,file=" + $vars),
      '-drive',("if=none,id=esp0,format=raw,snapshot=on,file=" + (Join-Path $root 'phase16-desktop-preview-esp.img')),
      '-device','ide-hd,drive=esp0,bus=ide.0,bootindex=0',
      '-drive',("if=none,id=scr0,format=raw,file=" + $data), '-device','virtio-blk-pci,drive=scr0',
      '-netdev','user,id=net0','-device','virtio-net-pci,netdev=net0', '-device','virtio-rng-pci',
      '-device','virtio-keyboard-pci', '-device','virtio-tablet-pci', '-display','sdl',
      '-chardev',("stdio,id=con0,signal=off,logfile=" + $serial), '-serial','chardev:con0', '-no-reboot')
    & $qemu @argsQemu
    $code = $LASTEXITCODE
    Write-Host "QEMU exit code: $code. Serial log: $serial"
    if ($code -ne 0) { exit $code }
} finally {
    Stop-Job $tcp,$udp -ErrorAction SilentlyContinue
    Remove-Job $tcp,$udp -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $tcpReady,$udpReady -ErrorAction SilentlyContinue
}
