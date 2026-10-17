# Installs the newest release of `best` on Windows: the archive is taken from GitHub, checked against the release's
# SHA256SUMS, and the binary put in $env:LOCALAPPDATA\Programs\best ($env:BEST_INSTALL_DIR chooses another place),
# which is added to the user's PATH.
#
#   irm https://raw.githubusercontent.com/behavioralstate/best-cli/main/install.ps1 | iex
#
# $env:BEST_VERSION = "0.1.0" pins a release instead of the newest.
$ErrorActionPreference = "Stop"

$repo = "behavioralstate/best-cli"
$dir = if ($env:BEST_INSTALL_DIR) { $env:BEST_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA "Programs\best" }
$arch = if ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -eq "Arm64") { "aarch64" } else { "x86_64" }
if ($arch -ne "x86_64") { throw "best: no release for Windows on $arch yet; build it with: cargo install --git https://github.com/$repo" }
$target = "$arch-pc-windows-msvc"

if ($env:BEST_VERSION) {
    $base = "https://github.com/$repo/releases/download/v$($env:BEST_VERSION)"
    $archive = "best-$($env:BEST_VERSION)-$target.zip"
} else {
    $base = "https://github.com/$repo/releases/latest/download"
    $archive = "best-$target.zip"
}

$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("best-install-" + [System.IO.Path]::GetRandomFileName())
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
    Write-Host "Downloading $base/$archive"
    Invoke-WebRequest -Uri "$base/$archive" -OutFile (Join-Path $tmp $archive)
    Invoke-WebRequest -Uri "$base/SHA256SUMS" -OutFile (Join-Path $tmp "SHA256SUMS")
    $line = Get-Content (Join-Path $tmp "SHA256SUMS") | Where-Object { $_ -match " \*?$([regex]::Escape($archive))$" }
    if (-not $line) { throw "best: $archive is not in SHA256SUMS" }
    $expected = ($line -split "\s+")[0].ToLowerInvariant()
    $actual = (Get-FileHash -Algorithm SHA256 (Join-Path $tmp $archive)).Hash.ToLowerInvariant()
    if ($expected -ne $actual) { throw "best: the checksum of $archive does not match SHA256SUMS" }
    Expand-Archive -Path (Join-Path $tmp $archive) -DestinationPath $tmp -Force
    New-Item -ItemType Directory -Path $dir -Force | Out-Null
    Copy-Item (Join-Path $tmp "best-$target\best.exe") (Join-Path $dir "best.exe") -Force
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if (($userPath -split ";") -notcontains $dir) {
        [Environment]::SetEnvironmentVariable("Path", "$userPath;$dir", "User")
        $env:Path = "$env:Path;$dir"
        Write-Host "Added $dir to your PATH (open a new terminal for other windows to see it)."
    }
    Write-Host "Installed $(& (Join-Path $dir 'best.exe') --version) in $dir"
    Write-Host "Next: best login <name> --manifest <address>"
} finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
