param([switch]$DebugBuild, [switch]$RunTests)

$ErrorActionPreference = 'Stop'
$project = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $project
try {
    $cargoCommand = Get-Command cargo.exe -ErrorAction SilentlyContinue
    if ($cargoCommand) {
        $cargoExecutable = $cargoCommand.Source
    } else {
        $cargoExecutable = Join-Path $project '.tools/cargo/bin/cargo.exe'
        if (-not (Test-Path -LiteralPath $cargoExecutable)) {
            throw 'Install Rust for Windows first: https://rust-lang.org/tools/install/'
        }
        $env:CARGO_HOME = Join-Path $project '.tools/cargo'
        $env:RUSTUP_HOME = Join-Path $project '.tools/rustup'
        $compiler = Get-ChildItem -LiteralPath (Join-Path $project '.tools') -Directory -Filter 'llvm-mingw-*' | Select-Object -First 1
        if (-not $compiler) { throw 'The local LLVM-MinGW compiler is missing.' }
        $compilerBin = Join-Path $compiler.FullName 'bin'
        $env:PATH = "$compilerBin;$env:CARGO_HOME/bin;$env:PATH"
        $env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = Join-Path $compilerBin 'x86_64-w64-mingw32-clang.exe'
        $env:CC_x86_64_pc_windows_gnu = $env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER
        $env:AR_x86_64_pc_windows_gnu = Join-Path $compilerBin 'llvm-ar.exe'
        # Use Rust's matching MinGW runtime libraries with the portable Clang driver.
        $env:RUSTFLAGS = "$env:RUSTFLAGS -C link-self-contained=yes".Trim()
    }
    $profile = if ($DebugBuild) { 'dev' } else { 'release' }
    # Windows PowerShell 5 treats redirected native stderr as error records.
    $ErrorActionPreference = 'Continue'
    if ($RunTests) {
        & $cargoExecutable test --locked 2>&1 | ForEach-Object { "$_" }
        $testExitCode = $LASTEXITCODE
        if ($testExitCode -ne 0) { throw "Windows tests failed ($testExitCode)." }
    }
    & $cargoExecutable build --locked --profile $profile 2>&1 | ForEach-Object { "$_" }
    $buildExitCode = $LASTEXITCODE
    $ErrorActionPreference = 'Stop'
    if ($buildExitCode -ne 0) { throw "Windows build failed ($buildExitCode)." }
    $profileDirectory = if ($DebugBuild) { 'debug' } else { 'release' }
    $output = "target/$profileDirectory/perfuse.exe"
    New-Item -ItemType Directory -Path 'target/windows' -Force | Out-Null
    Copy-Item -LiteralPath $output -Destination 'target/windows/perfuse.exe' -Force
    Copy-Item -LiteralPath 'LICENSE' -Destination 'target/windows/LICENSE.txt' -Force
    $library = Join-Path $project '.tools/libmpv/libmpv-2.dll'
    if (Test-Path -LiteralPath $library) {
        Copy-Item -LiteralPath $library -Destination 'target/windows/libmpv-2.dll' -Force
    }
    Write-Output "Windows executable: $(Join-Path $project 'target/windows/perfuse.exe')"
} finally {
    Pop-Location
}
