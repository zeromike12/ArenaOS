param(
    [string]$HostAddress = "127.0.0.1",
    [int]$Port = 4444,
    [string]$LogPath = "$PSScriptRoot\phase14-vcon.log"
)

$deadline = [DateTime]::UtcNow.AddSeconds(90)
$client = $null
while ([DateTime]::UtcNow -lt $deadline) {
    $candidate = [System.Net.Sockets.TcpClient]::new()
    try {
        $candidate.Connect($HostAddress, $Port)
        $client = $candidate
        break
    } catch {
        $candidate.Dispose()
        Start-Sleep -Milliseconds 200
    }
}
if ($null -eq $client -or -not $client.Connected) {
    throw "QEMU virtio-console socket did not open at ${HostAddress}:$Port"
}

$stream = $client.GetStream()
$stream.ReadTimeout = 1000
$buffer = [byte[]]::new(4096)
$seen = [System.Text.StringBuilder]::new()
$end = [DateTime]::UtcNow.AddMinutes(2)
$answered = $false
try {
    while ([DateTime]::UtcNow -lt $end -and -not $answered) {
        try {
            $count = $stream.Read($buffer, 0, $buffer.Length)
            if ($count -eq 0) { break }
            [void]$seen.Append([System.Text.Encoding]::ASCII.GetString($buffer, 0, $count))
            if ($seen.ToString().Contains("contest: hello from ArenaOS")) {
                $reply = [System.Text.Encoding]::ASCII.GetBytes("host-says-hello`n")
                $stream.Write($reply, 0, $reply.Length)
                $stream.Flush()
                $answered = $true
            }
        } catch [System.IO.IOException] {
            # ReadTimeout: keep waiting for the marker without busy polling.
        }
    }
} finally {
    $seen.ToString() | Set-Content -Encoding ascii -Path $LogPath
    $stream.Dispose()
    $client.Dispose()
}
if (-not $answered) {
    throw "ArenaOS did not send the expected virtio-console test marker"
}
Write-Host "ArenaOS virtio-console handshake passed; capture: $LogPath"
