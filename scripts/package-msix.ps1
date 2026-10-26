<#
.SYNOPSIS
  Builds Microsoft Store packages (MSIX) for Scratchpad.

.DESCRIPTION
  Per architecture:
    scripts/package-msix.ps1 -Target x86_64-pc-windows-msvc -OutDir dist-store
  Then bundle every .msix in a folder into one .msixbundle plus the .msixupload that
  Partner Center and `msstore publish` accept:
    scripts/package-msix.ps1 -Bundle -OutDir dist-store

  The package identity must match Partner Center → Product management → Product identity:
    MSIX_IDENTITY_NAME            Package/Identity/Name           (e.g. 12345ScaleXY.Scratchpad)
    MSIX_PUBLISHER                Package/Identity/Publisher      (e.g. CN=XXXXXXXX-XXXX-...)
    MSIX_PUBLISHER_DISPLAY_NAME   Package/Properties/PublisherDisplayName
    MSIX_DISPLAY_NAME             the reserved app name (default Scratchpad)
  Store packages are left unsigned: the Store signs them during certification.
  See docs/windows-store.md.
#>
param(
    [string]$Target,
    [string]$OutDir = "dist-store",
    [switch]$Bundle
)
$ErrorActionPreference = "Stop"
$Root = Resolve-Path (Join-Path $PSScriptRoot "..")

$Version = & "$PSScriptRoot\version.ps1"
# MSIX versions are four numbers and the Store requires the last one to be 0.
$Core = ($Version -split '[-+]')[0]
$MsixVersion = "$Core.0"

function Find-SdkTool([string]$Name) {
    $cmd = Get-Command $Name -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    $hostArch = if ($env:PROCESSOR_ARCHITECTURE -eq "ARM64") { "arm64" } else { "x64" }
    $kits = "${env:ProgramFiles(x86)}\Windows Kits\10\bin"
    foreach ($arch in @($hostArch, "x64", "x86")) {
        $found = Get-ChildItem $kits -Recurse -Filter $Name -ErrorAction SilentlyContinue |
            Where-Object { $_.FullName -match "\\$arch\\" } | Sort-Object FullName -Descending | Select-Object -First 1
        if ($found) { return $found.FullName }
    }
    throw "$Name not found (install the Windows 10/11 SDK)"
}

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$OutDir = Resolve-Path $OutDir
$MakeAppx = Find-SdkTool "makeappx.exe"

if ($Bundle) {
    $packages = Get-ChildItem $OutDir -Filter "*.msix"
    if (-not $packages) { throw "no .msix packages in $OutDir" }
    $staging = Join-Path ([IO.Path]::GetTempPath()) "scratchpad-msix-bundle"
    Remove-Item $staging -Recurse -Force -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Path $staging | Out-Null
    $packages | Copy-Item -Destination $staging

    $bundle = Join-Path $OutDir "scratchpad-$Version.msixbundle"
    & $MakeAppx bundle /d $staging /p $bundle /bv $MsixVersion /o
    if ($LASTEXITCODE -ne 0) { throw "makeappx bundle failed" }

    # An .msixupload is a zip holding the bundle (and optionally symbols).
    $zip = Join-Path $OutDir "scratchpad-$Version.zip"
    Compress-Archive -Path $bundle -DestinationPath $zip -Force
    Move-Item $zip (Join-Path $OutDir "scratchpad-$Version.msixupload") -Force
    Write-Host "Bundled $($packages.Count) packages into $bundle"
    return
}

$Arch = switch ($Target) {
    "x86_64-pc-windows-msvc" { "x64" }
    "i686-pc-windows-msvc" { "x86" }
    "aarch64-pc-windows-msvc" { "arm64" }
    default { throw "unsupported target '$Target'" }
}
$Exe = Join-Path $Root "target\$Target\release\scratchpad.exe"
if (-not (Test-Path $Exe)) { throw "build $Exe first (cargo build --release --target $Target -p scratchpad)" }

$Stage = Join-Path ([IO.Path]::GetTempPath()) "scratchpad-msix-$Arch"
Remove-Item $Stage -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Path $Stage | Out-Null
Copy-Item $Exe (Join-Path $Stage "scratchpad.exe")
Copy-Item (Join-Path $Root "LICENSE") $Stage
Copy-Item (Join-Path $Root "packaging\windows\msix\Assets") (Join-Path $Stage "Assets") -Recurse

function Escape-Xml([string]$Value) { [Security.SecurityElement]::Escape($Value) }
$values = @{
    "{{IDENTITY_NAME}}"          = if ($env:MSIX_IDENTITY_NAME) { $env:MSIX_IDENTITY_NAME } else { "ScaleXY.Scratchpad" }
    "{{PUBLISHER}}"              = if ($env:MSIX_PUBLISHER) { $env:MSIX_PUBLISHER } else { "CN=ScaleXY" }
    "{{PUBLISHER_DISPLAY_NAME}}" = if ($env:MSIX_PUBLISHER_DISPLAY_NAME) { $env:MSIX_PUBLISHER_DISPLAY_NAME } else { "ScaleXY" }
    "{{DISPLAY_NAME}}"           = if ($env:MSIX_DISPLAY_NAME) { $env:MSIX_DISPLAY_NAME } else { "Scratchpad" }
    "{{VERSION}}"                = $MsixVersion
    "{{ARCH}}"                   = $Arch
}
$manifest = Get-Content (Join-Path $Root "packaging\windows\msix\AppxManifest.xml") -Raw
foreach ($key in $values.Keys) { $manifest = $manifest.Replace($key, (Escape-Xml $values[$key])) }
if ($manifest -match "\{\{") { throw "unfilled placeholder in AppxManifest.xml" }
Set-Content -Path (Join-Path $Stage "AppxManifest.xml") -Value $manifest -Encoding UTF8

# Index the scale/target-size asset variants so Windows picks the right one.
$MakePri = Find-SdkTool "makepri.exe"
$priConfig = Join-Path ([IO.Path]::GetTempPath()) "scratchpad-priconfig.xml"
& $MakePri createconfig /cf $priConfig /dq en-US /pv 10.0.0 /o
if ($LASTEXITCODE -ne 0) { throw "makepri createconfig failed" }
& $MakePri new /pr $Stage /cf $priConfig /mn (Join-Path $Stage "AppxManifest.xml") /of (Join-Path $Stage "resources.pri") /o
if ($LASTEXITCODE -ne 0) { throw "makepri new failed" }

$Msix = Join-Path $OutDir "scratchpad-$Version-$Arch.msix"
& $MakeAppx pack /d $Stage /p $Msix /o
if ($LASTEXITCODE -ne 0) { throw "makeappx pack failed" }
Write-Host "Packaged $Msix ($MsixVersion, $Arch)"
