[CmdletBinding()]
param(
    [string]$Qemu,
    [string]$OvmfCode,
    [string]$OvmfVars,
    [string]$Python = 'python',
    [string]$OutputDirectory
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
if (-not [Environment]::Is64BitOperatingSystem -or $env:PROCESSOR_ARCHITECTURE -ne 'AMD64') {
    Write-Warning 'The WHPX runner requires 64-bit Windows on an AMD64 host; the runner will emit BLOCKED without a guest boot.'
}
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$driver = Join-Path $PSScriptRoot 'run_external.py'
$verifier = Join-Path $PSScriptRoot 'verify_receipt.py'
$efi = Join-Path $repoRoot 'artifacts\arena-boot.efi'
$esp = Join-Path $repoRoot 'artifacts\arena-esp.img'
if (-not (Test-Path $efi)) { $efi = Join-Path $repoRoot 'build\phase15-strict-efi-6586a0a.efi' }
if (-not (Test-Path $esp)) { $esp = Join-Path $repoRoot 'build\phase15-strict-esp-6586a0a.img' }

if (-not $Qemu) {
    $found = Get-Command 'qemu-system-x86_64.exe' -ErrorAction SilentlyContinue
    if (-not $found) { $found = Get-Command 'qemu-system-x86_64' -ErrorAction SilentlyContinue }
    if ($found) { $Qemu = $found.Source }
}
if (-not $Qemu) { $Qemu = 'qemu-system-x86_64.exe' }

if (-not $OvmfCode -and $env:ARENA_OVMF_CODE) { $OvmfCode = $env:ARENA_OVMF_CODE }
if (-not $OvmfVars -and $env:ARENA_OVMF_VARS) { $OvmfVars = $env:ARENA_OVMF_VARS }
if ((Test-Path $Qemu -PathType Leaf)) {
    $qemuDir = Split-Path -Parent (Resolve-Path $Qemu).Path
    $fwDirs = @((Join-Path $qemuDir 'share'), (Join-Path $qemuDir 'share\qemu'), $qemuDir)
    if (-not $OvmfCode) {
        foreach ($dir in $fwDirs) {
            foreach ($name in @('edk2-x86_64-code.fd', 'OVMF_CODE.fd')) {
                $candidate = Join-Path $dir $name
                if (Test-Path $candidate -PathType Leaf) { $OvmfCode = $candidate; break }
            }
            if ($OvmfCode) { break }
        }
    }
    if (-not $OvmfVars) {
        foreach ($dir in $fwDirs) {
            foreach ($name in @('edk2-i386-vars.fd', 'OVMF_VARS.fd')) {
                $candidate = Join-Path $dir $name
                if (Test-Path $candidate -PathType Leaf) { $OvmfVars = $candidate; break }
            }
            if ($OvmfVars) { break }
        }
    }
}

Write-Host "QEMU executable: $Qemu"
if (Test-Path $Qemu) {
    $accelText = (& $Qemu '-accel' 'help' 2>&1 | Out-String)
    Write-Host 'QEMU accelerators:'
    Write-Host $accelText.Trim()
    if ($accelText -notmatch '(?i)\bwhpx\b') {
        Write-Warning 'This QEMU build does not advertise WHPX; the runner will emit BLOCKED without booting a guest.'
    }
} else {
    Write-Warning 'QEMU was not found at the selected path; the runner will emit BLOCKED without booting a guest.'
}

if (-not $OvmfCode -or -not (Test-Path $OvmfCode -PathType Leaf)) {
    Write-Warning 'Could not locate OVMF code firmware. The runner will emit BLOCKED without booting a guest.'
}
if (-not $OvmfVars -or -not (Test-Path $OvmfVars -PathType Leaf)) {
    Write-Warning 'Could not locate OVMF vars template. The runner will emit BLOCKED without booting a guest.'
}

if (-not $OutputDirectory) {
    $stamp = (Get-Date).ToUniversalTime().ToString('yyyyMMddTHHmmssfffZ')
    $OutputDirectory = Join-Path $repoRoot "build\phase15-external-$stamp"
}

$runnerArgs = @(
    $driver,
    '--backend', 'whpx',
    '--qemu', $Qemu,
    '--efi', $efi,
    '--esp', $esp,
    '--output-dir', $OutputDirectory
)
if ($OvmfCode) { $runnerArgs += @('--ovmf-code', $OvmfCode) }
if ($OvmfVars) { $runnerArgs += @('--ovmf-vars', $OvmfVars) }
Write-Host 'Starting strict Phase 15 test with explicit -accel whpx.'
& $Python @runnerArgs
$runnerExit = $LASTEXITCODE

$receipt = Join-Path $OutputDirectory 'receipt.json'
if (Test-Path $receipt) {
    Write-Host 'Checking the captured qualification receipt.'
    & $Python $verifier '--receipt' $receipt
    $verifyExit = $LASTEXITCODE
} else {
    $verifyExit = 2
    Write-Error "The runner did not create a receipt at $receipt"
}

if ($runnerExit -ne 0) { exit $runnerExit }
if ($verifyExit -ne 0) { exit $verifyExit }
Write-Host "PHASE 15 strict WHPX guest test passed. Receipt: $receipt"
exit 0
