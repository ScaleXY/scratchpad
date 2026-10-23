<#
.SYNOPSIS
  Builds, signs and packages Scratchpad for one Windows target.

.EXAMPLE
  scripts/package-windows.ps1 -Target x86_64-pc-windows-msvc

  Targets: x86_64-pc-windows-msvc, i686-pc-windows-msvc, aarch64-pc-windows-msvc

  Signing (optional) uses either:
    * WINDOWS_CERTIFICATE (base64 .pfx) + WINDOWS_CERTIFICATE_PASSWORD, signed with signtool, or
    * Azure Trusted Signing: AZURE_TENANT_ID, AZURE_CLIENT_ID, AZURE_CLIENT_SECRET,
      AZURE_SIGNING_ENDPOINT, AZURE_SIGNING_ACCOUNT, AZURE_CERTIFICATE_PROFILE
  Without either, artifacts are left unsigned (SmartScreen will warn).
#>
param(
    [Parameter(Mandatory = $true)][string]$Target,
    [string]$OutDir = "dist"
)
$ErrorActionPreference = "Stop"
$Root = Resolve-Path (Join-Path $PSScriptRoot "..")

$Arch = switch ($Target) {
    "x86_64-pc-windows-msvc" { "x64" }
    "i686-pc-windows-msvc" { "x86" }
    "aarch64-pc-windows-msvc" { "arm64" }
    default { throw "unsupported target $Target" }
}

$Metadata = cargo metadata --no-deps --format-version 1 --manifest-path "$Root\Cargo.toml" | ConvertFrom-Json
$Version = ($Metadata.packages | Where-Object { $_.name -eq "scratchpad" }).version

$Exe = Join-Path $Root "target\$Target\release\scratchpad.exe"
if (-not (Test-Path $Exe)) {
    cargo build --release --locked --target $Target -p scratchpad --manifest-path "$Root\Cargo.toml"
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
}

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$OutDir = Resolve-Path $OutDir

function Find-SignTool {
    $cmd = Get-Command signtool.exe -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    $kits = "${env:ProgramFiles(x86)}\Windows Kits\10\bin"
    $found = Get-ChildItem $kits -Recurse -Filter signtool.exe -ErrorAction SilentlyContinue |
        Where-Object { $_.FullName -match "\\x64\\" } | Sort-Object FullName -Descending | Select-Object -First 1
    if (-not $found) { throw "signtool.exe not found" }
    return $found.FullName
}

function Invoke-Sign([string]$Path) {
    if ($env:WINDOWS_CERTIFICATE) {
        $pfx = Join-Path $env:RUNNER_TEMP "scratchpad-signing.pfx"
        if (-not $env:RUNNER_TEMP) { $pfx = Join-Path ([IO.Path]::GetTempPath()) "scratchpad-signing.pfx" }
        [IO.File]::WriteAllBytes($pfx, [Convert]::FromBase64String($env:WINDOWS_CERTIFICATE))
        try {
            & (Find-SignTool) sign /fd SHA256 /td SHA256 /tr "http://timestamp.digicert.com" `
                /f $pfx /p "$env:WINDOWS_CERTIFICATE_PASSWORD" /d "Scratchpad" $Path
            if ($LASTEXITCODE -ne 0) { throw "signtool failed for $Path" }
        } finally {
            Remove-Item $pfx -Force -ErrorAction SilentlyContinue
        }
        & (Find-SignTool) verify /pa $Path
    } elseif ($env:AZURE_SIGNING_ENDPOINT) {
        # Requires the Microsoft.Trusted.Signing.Client dlib (installed by the release workflow).
        $metadata = Join-Path ([IO.Path]::GetTempPath()) "scratchpad-trusted-signing.json"
        @{ Endpoint = $env:AZURE_SIGNING_ENDPOINT; CodeSigningAccountName = $env:AZURE_SIGNING_ACCOUNT; CertificateProfileName = $env:AZURE_CERTIFICATE_PROFILE } |
            ConvertTo-Json | Set-Content $metadata
        & (Find-SignTool) sign /v /fd SHA256 /tr "http://timestamp.acs.microsoft.com" /td SHA256 `
            /dlib $env:AZURE_SIGNING_DLIB /dmdf $metadata $Path
        if ($LASTEXITCODE -ne 0) { throw "Trusted Signing failed for $Path" }
    } else {
        Write-Warning "No signing credentials configured: $Path is unsigned"
    }
}

Invoke-Sign $Exe

$Name = "scratchpad-$Version-windows-$Arch"
$Stage = Join-Path ([IO.Path]::GetTempPath()) $Name
Remove-Item $Stage -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Path $Stage | Out-Null
Copy-Item $Exe (Join-Path $Stage "scratchpad.exe")
Copy-Item (Join-Path $Root "README.md") $Stage
Compress-Archive -Path "$Stage\*" -DestinationPath (Join-Path $OutDir "$Name.zip") -Force

# Installer (Inno Setup is preinstalled on GitHub's Windows images; `choco install innosetup` otherwise).
$Iscc = Get-Command iscc.exe -ErrorAction SilentlyContinue
if (-not $Iscc) { $Iscc = Get-Item "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe" -ErrorAction SilentlyContinue }
if ($Iscc) {
    $IsccPath = if ($Iscc.Source) { $Iscc.Source } else { $Iscc.FullName }
    & $IsccPath "/DAppVersion=$Version" "/DArch=$Arch" "/DSourceExe=$Exe" "/DOutputDir=$OutDir" `
        (Join-Path $Root "packaging\windows\installer.iss")
    if ($LASTEXITCODE -ne 0) { throw "Inno Setup failed" }
    Invoke-Sign (Join-Path $OutDir "$Name-setup.exe")
} else {
    Write-Warning "Inno Setup not found: skipping installer"
}
Write-Host "Packaged $Name"
