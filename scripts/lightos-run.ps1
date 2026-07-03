# LightOS launcher for Windows (PowerShell).
#
# Boots LightOS in QEMU from a release bundle. Place this next to the
# kernel (lightos) and root filesystem (rootfs.img), then:
#
#     .\run.ps1
#
# Requires QEMU for Windows (https://qemu.weilnetz.de/w64/) with
# qemu-system-riscv64 on PATH. Exit the guest with Ctrl-A then x.
#
# Options: -Memory <MiB> (default 128), -Smp <N> (default 4).
param(
    [int]$Memory = 128,
    [int]$Smp = 4
)

$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$kernel = Join-Path $here "lightos"
$disk = Join-Path $here "rootfs.img"

$qemu = Get-Command qemu-system-riscv64 -ErrorAction SilentlyContinue
if (-not $qemu) {
    Write-Error @"
qemu-system-riscv64 not found.

Install QEMU for Windows from https://qemu.weilnetz.de/w64/ and make
sure its folder (e.g. C:\Program Files\qemu) is on your PATH.
"@
    exit 1
}

foreach ($f in @($kernel, $disk)) {
    if (-not (Test-Path $f)) {
        Write-Error "missing $f (is this a complete bundle?)"
        exit 1
    }
}

Write-Host "Booting LightOS (mem ${Memory}M, $Smp harts). Exit with Ctrl-A x."
& qemu-system-riscv64 `
    -machine virt -cpu rv64 -smp $Smp -m "${Memory}M" `
    -bios none -kernel $kernel `
    -drive "file=$disk,format=raw,id=hd0" `
    -device virtio-blk-device,drive=hd0 `
    -serial stdio -display none
