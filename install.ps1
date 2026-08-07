param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^v[0-9][0-9A-Za-z._-]*$')]
    [string]$Version,
    [string]$Prefix = ""
)

$ErrorActionPreference = "Stop"
$Repository = "ovaso/batch-git-rs"
$Target = "x86_64-pc-windows-msvc"
$Archive = "batch-git-$Target.tar.gz"

if (-not [Environment]::Is64BitOperatingSystem) {
    throw "batch-git currently publishes only a 64-bit Windows archive"
}
if ([string]::IsNullOrWhiteSpace($Prefix)) {
    $Prefix = Join-Path $env:LOCALAPPDATA "Programs\batch-git"
}

$TemporaryDirectory = Join-Path ([IO.Path]::GetTempPath()) ("batch-git-install-" + [Guid]::NewGuid())
New-Item -ItemType Directory -Path $TemporaryDirectory | Out-Null
try {
    $BaseUrl = "https://github.com/$Repository/releases/download/$Version"
    $ArchivePath = Join-Path $TemporaryDirectory $Archive
    $ChecksumPath = "$ArchivePath.sha256"
    Invoke-WebRequest -Uri "$BaseUrl/$Archive" -OutFile $ArchivePath
    Invoke-WebRequest -Uri "$BaseUrl/$Archive.sha256" -OutFile $ChecksumPath

    $Expected = ((Get-Content -Raw $ChecksumPath).Trim() -split '\s+')[0].ToLowerInvariant()
    if ($Expected -notmatch '^[0-9a-f]{64}$') {
        throw "release checksum is malformed"
    }
    $Actual = (Get-FileHash -Algorithm SHA256 $ArchivePath).Hash.ToLowerInvariant()
    if ($Actual -ne $Expected) {
        throw "SHA-256 verification failed"
    }

    tar -xzf $ArchivePath -C $TemporaryDirectory
    if ($LASTEXITCODE -ne 0) {
        throw "failed to extract release archive"
    }
    $Package = Join-Path $TemporaryDirectory "batch-git-$Target"
    $Binary = Join-Path $Package "batch-git.exe"
    if (-not (Test-Path -PathType Leaf $Binary)) {
        throw "archive does not contain batch-git.exe"
    }

    $BinDirectory = Join-Path $Prefix "bin"
    $CompletionDirectory = Join-Path $Prefix "share\batch-git\completions"
    New-Item -ItemType Directory -Force -Path $BinDirectory, $CompletionDirectory | Out-Null
    Copy-Item $Binary (Join-Path $BinDirectory "batch-git.exe") -Force
    Copy-Item (Join-Path $Package "completions\batch-git.ps1") $CompletionDirectory -Force

    Write-Host "installed batch-git $Version to $BinDirectory\batch-git.exe"
    Write-Host "add $BinDirectory to PATH; dot-source $CompletionDirectory\batch-git.ps1 for PowerShell completion"
}
finally {
    Remove-Item -Recurse -Force $TemporaryDirectory -ErrorAction SilentlyContinue
}
