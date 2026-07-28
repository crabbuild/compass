param(
    [string]$CompassBinary
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$RepoRoot = Split-Path -Parent $PSScriptRoot
$TestRoot = Join-Path ([System.IO.Path]::GetTempPath()) "compass-install-test-$([guid]::NewGuid())"
$ReleaseDir = Join-Path $TestRoot "release"
$Server = $null
$SavedBaseUrl = $env:COMPASS_RELEASE_BASE_URL
$SavedInstallDir = $env:COMPASS_INSTALL_DIR
$SavedArchitecture = $env:COMPASS_INSTALL_ARCH

function Write-FixtureArchive {
    param(
        [string]$Target,
        [string]$Payload
    )
    $Name = "compass-$Target"
    $Bundle = Join-Path $TestRoot $Name
    New-Item -ItemType Directory -Force -Path $Bundle | Out-Null
    $Executable = Join-Path $Bundle "compass.exe"
    if ($CompassBinary) {
        Copy-Item -LiteralPath $CompassBinary -Destination $Executable
    } else {
        [System.IO.File]::WriteAllText($Executable, $Payload)
    }
    $Archive = Join-Path $ReleaseDir "$Name.tar.gz"
    & tar.exe -czf $Archive -C $TestRoot $Name
    if ($LASTEXITCODE -ne 0) {
        throw "failed to create fixture archive for $Target"
    }
    $Hash = (Get-FileHash $Archive -Algorithm SHA256).Hash.ToLowerInvariant()
    [System.IO.File]::WriteAllText("$Archive.sha256", "$Hash  $Name.tar.gz`n")
    Remove-Item -LiteralPath $Bundle -Recurse -Force
    return $Executable
}

try {
    New-Item -ItemType Directory -Force -Path $ReleaseDir | Out-Null
    Write-FixtureArchive "x86_64-pc-windows-msvc" "fixture-x64" | Out-Null
    Write-FixtureArchive "aarch64-pc-windows-msvc" "fixture-arm64" | Out-Null

    $Listener = [System.Net.Sockets.TcpListener]::new(
        [System.Net.IPAddress]::Loopback,
        0
    )
    $Listener.Start()
    $Port = ([System.Net.IPEndPoint]$Listener.LocalEndpoint).Port
    $Listener.Stop()
    $Server = Start-Process python -ArgumentList @(
        "-m", "http.server", "$Port", "--bind", "127.0.0.1"
    ) -WorkingDirectory $ReleaseDir -PassThru -WindowStyle Hidden
    $BaseUrl = "http://127.0.0.1:$Port"
    for ($Attempt = 0; $Attempt -lt 50; $Attempt++) {
        try {
            Invoke-WebRequest -UseBasicParsing -Uri $BaseUrl | Out-Null
            break
        } catch {
            if ($Attempt -eq 49) {
                throw "fixture release server did not start"
            }
            Start-Sleep -Milliseconds 100
        }
    }

    foreach ($Case in @(
        @{ Architecture = "x64"; Target = "x86_64-pc-windows-msvc" },
        @{ Architecture = "arm64"; Target = "aarch64-pc-windows-msvc" }
    )) {
        $InstallDir = Join-Path $TestRoot "install-$($Case.Target)"
        $env:COMPASS_RELEASE_BASE_URL = $BaseUrl
        $env:COMPASS_INSTALL_DIR = $InstallDir
        $env:COMPASS_INSTALL_ARCH = $Case.Architecture
        & (Join-Path $RepoRoot "scripts\install.ps1")
        $Installed = Join-Path $InstallDir "compass.exe"
        if (-not (Test-Path -LiteralPath $Installed -PathType Leaf)) {
            throw "installer did not publish $Installed"
        }
        if ($CompassBinary) {
            $ExpectedHash = (Get-FileHash $CompassBinary -Algorithm SHA256).Hash
            $ActualHash = (Get-FileHash $Installed -Algorithm SHA256).Hash
            if ($ActualHash -ne $ExpectedHash) {
                throw "installed fixture does not match the source binary"
            }
        }
    }

    $BadChecksum = Join-Path $ReleaseDir `
        "compass-x86_64-pc-windows-msvc.tar.gz.sha256"
    [System.IO.File]::WriteAllText(
        $BadChecksum,
        "$("0" * 64)  compass-x86_64-pc-windows-msvc.tar.gz`n"
    )
    $env:COMPASS_INSTALL_ARCH = "x64"
    $env:COMPASS_INSTALL_DIR = Join-Path $TestRoot "checksum-must-fail"
    $Failed = $false
    try {
        & (Join-Path $RepoRoot "scripts\install.ps1")
    } catch {
        $Failed = $true
    }
    if (-not $Failed) {
        throw "installer accepted a bad checksum"
    }
    if (Test-Path -LiteralPath (Join-Path $env:COMPASS_INSTALL_DIR "compass.exe")) {
        throw "installer published a binary after checksum failure"
    }
    Write-Output "PowerShell installer tests passed"
} finally {
    if ($Server -and -not $Server.HasExited) {
        Stop-Process -Id $Server.Id -Force -ErrorAction SilentlyContinue
    }
    $env:COMPASS_RELEASE_BASE_URL = $SavedBaseUrl
    $env:COMPASS_INSTALL_DIR = $SavedInstallDir
    $env:COMPASS_INSTALL_ARCH = $SavedArchitecture
    Remove-Item -LiteralPath $TestRoot -Recurse -Force -ErrorAction SilentlyContinue
}
