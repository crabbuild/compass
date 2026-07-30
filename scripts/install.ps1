$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$Repository = if ($env:COMPASS_REPOSITORY) {
    $env:COMPASS_REPOSITORY
} else {
    "crabbuild/compass"
}
$ReleaseBaseUrl = if ($env:COMPASS_RELEASE_BASE_URL) {
    $env:COMPASS_RELEASE_BASE_URL.TrimEnd("/")
} else {
    "https://github.com/$Repository/releases/latest/download"
}
$InstallDir = if ($env:COMPASS_INSTALL_DIR) {
    $env:COMPASS_INSTALL_DIR
} else {
    Join-Path $HOME ".local\bin"
}
$ArchitectureName = if ($env:COMPASS_INSTALL_ARCH) {
    $env:COMPASS_INSTALL_ARCH
} else {
    try {
        [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
    } catch {
        $env:PROCESSOR_ARCHITECTURE
    }
}
$Target = switch ($ArchitectureName.ToLowerInvariant()) {
    "x64" { "x86_64-pc-windows-msvc" }
    "amd64" { "x86_64-pc-windows-msvc" }
    "arm64" { "aarch64-pc-windows-msvc" }
    default { throw "unsupported Windows architecture: $ArchitectureName" }
}

$Name = "compass-$Target"
$Archive = "$Name.tar.gz"
$Checksum = "$Archive.sha256"
$Temporary = Join-Path ([System.IO.Path]::GetTempPath()) "compass-install-$([guid]::NewGuid())"

try {
    New-Item -ItemType Directory -Path $Temporary | Out-Null
    Invoke-WebRequest -UseBasicParsing -Uri "$ReleaseBaseUrl/$Archive" `
        -OutFile (Join-Path $Temporary $Archive)
    Invoke-WebRequest -UseBasicParsing -Uri "$ReleaseBaseUrl/$Checksum" `
        -OutFile (Join-Path $Temporary $Checksum)

    $Expected = ((Get-Content (Join-Path $Temporary $Checksum) -Raw).Trim() `
        -split "\s+")[0].ToLowerInvariant()
    if ($Expected -notmatch "^[0-9a-f]{64}$") {
        throw "invalid SHA-256 file for $Archive"
    }
    $Actual = (Get-FileHash (Join-Path $Temporary $Archive) -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($Actual -ne $Expected) {
        throw "checksum verification failed for $Archive"
    }

    & tar.exe -xzf (Join-Path $Temporary $Archive) -C $Temporary
    if ($LASTEXITCODE -ne 0) {
        throw "failed to extract $Archive"
    }
    $Source = Join-Path $Temporary "$Name\compass.exe"
    if (-not (Test-Path -LiteralPath $Source -PathType Leaf)) {
        throw "release archive does not contain $Name\compass.exe"
    }

    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    $Destination = Join-Path $InstallDir "compass.exe"
    $Staged = Join-Path $InstallDir "compass.exe.new"
    Copy-Item -LiteralPath $Source -Destination $Staged -Force
    Move-Item -LiteralPath $Staged -Destination $Destination -Force
    Write-Output "Installed Compass to $Destination"
    if (($env:PATH -split [System.IO.Path]::PathSeparator) -notcontains $InstallDir) {
        Write-Output "Add $InstallDir to PATH before running compass from a new shell."
    }
} finally {
    Remove-Item -LiteralPath $Temporary -Recurse -Force -ErrorAction SilentlyContinue
}
