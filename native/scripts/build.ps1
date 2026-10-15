# Build and verify the Shuttle native workspace on Windows (PowerShell).
#
#   .\native\scripts\build.ps1               dev build + verify
#   .\native\scripts\build.ps1 -Release      release build + verify
#   .\native\scripts\build.ps1 -SkipVerify   build only
#
# Needs: Rust (rustup), and LLVM for clang + wasm-ld
# (installer: https://github.com/llvm/llvm-project/releases).
param(
    [switch]$Release,
    [switch]$SkipVerify
)
$ErrorActionPreference = 'Stop'

$Native = Resolve-Path (Join-Path $PSScriptRoot '..')
Set-Location $Native

$Profile = if ($Release) { 'release' } else { 'dev' }
$ProfileDir = if ($Release) { 'release' } else { 'debug' }

function Need($cmd, $hint) {
    if (-not (Get-Command $cmd -ErrorAction SilentlyContinue)) {
        throw "'$cmd' not found. $hint"
    }
}

# The LLVM installer does not always add itself to PATH.
$llvm = Join-Path $env:ProgramFiles 'LLVM\bin'
if (-not (Get-Command clang -ErrorAction SilentlyContinue) -and (Test-Path $llvm)) {
    $env:PATH = "$llvm;$env:PATH"
}

Write-Host '== guests (C -> wasm32)'
Need clang 'Install LLVM from https://github.com/llvm/llvm-project/releases'
Need wasm-ld 'wasm-ld comes with the LLVM installer'
New-Item -ItemType Directory -Force -Path 'guests\out' | Out-Null
foreach ($prog in 'bios', 'os', 'child') {
    # same flags as guests/build.sh
    & clang --target=wasm32 -O2 -nostdlib -ffreestanding `
        -Wall -Wno-unused-function `
        '-Wl,--no-entry' '-Wl,--export=_start' '-Wl,-z,stack-size=65536' `
        -o "guests\out\$prog.wasm" "guests\$prog.c"
    if ($LASTEXITCODE -ne 0) { throw "clang failed for $prog.c" }
}

Need cargo 'Install Rust from https://rustup.rs'
Write-Host "== cargo build ($Profile)"
& cargo build --locked --profile $Profile
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

if (-not $SkipVerify) {
    Write-Host '== verify (capture/restore at every tick)'
    & ".\target\$ProfileDir\shuttle-runner.exe" verify `
        guests\out\bios.wasm guests\out\os.wasm guests\out\child.wasm
    if ($LASTEXITCODE -ne 0) { throw 'verify failed' }
}
