# Prints the version of a build, from its git tag. Same rules as scripts/version.sh.
# Not "Stop": Windows PowerShell turns git's stderr into an error when there is no tag.
$ErrorActionPreference = "Continue"
if ($env:SCRATCHPAD_VERSION) {
    $env:SCRATCHPAD_VERSION -replace '^v', ''
    return
}
$describe = git -C $PSScriptRoot describe --tags --long --match "v[0-9]*" 2>$null
if ($LASTEXITCODE -eq 0 -and $describe -match '^v(.+)-(\d+)-(g[0-9a-f]+)$') {
    if ($Matches[2] -eq "0") { $Matches[1] } else { "$($Matches[1])+$($Matches[2]).$($Matches[3])" }
} else {
    "0.0.0"
}
