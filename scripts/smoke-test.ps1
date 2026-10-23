# Runs a built scratchpad.exe headless and checks SMTP capture end to end through the web API.
#   scripts/smoke-test.ps1 -Binary path\to\scratchpad.exe
param([Parameter(Mandatory = $true)][string]$Binary)
$ErrorActionPreference = "Stop"
$Data = Join-Path ([IO.Path]::GetTempPath()) ("scratchpad-smoke-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $Data | Out-Null
$Base = "http://127.0.0.1:28025"
$Auth = @{ Authorization = "Basic " + [Convert]::ToBase64String([Text.Encoding]::ASCII.GetBytes("smoke:test")) }

& $Binary --version
$process = Start-Process -FilePath $Binary -PassThru -NoNewWindow `
    -ArgumentList "--headless", "--data-dir", "`"$Data`"", "--smtp-port", "22525", "--web-port", "28025", "--web-auth", "smoke:test" `
    -RedirectStandardOutput (Join-Path $Data "out.log") -RedirectStandardError (Join-Path $Data "err.log")
try {
    $ready = $false
    for ($i = 0; $i -lt 50 -and -not $ready; $i++) {
        try { Invoke-RestMethod "$Base/healthz" | Out-Null; $ready = $true } catch { Start-Sleep -Milliseconds 200 }
    }
    if (-not $ready) { Get-Content (Join-Path $Data "err.log"); throw "server did not start" }
    & $Binary --healthcheck --data-dir "$Data" --web-port 28025
    if ($LASTEXITCODE -ne 0) { throw "--healthcheck failed" }

    try { Invoke-RestMethod "$Base/api/status" | Out-Null; throw "expected 401 without credentials" }
    catch { if ($_.Exception.Response.StatusCode.value__ -ne 401) { throw } }

    Invoke-RestMethod -Method Post -Uri "$Base/api/test-email" -Headers $Auth -ContentType "application/json" -Body '{"mailbox":"Smoke"}' | Out-Null
    $count = 0
    for ($i = 0; $i -lt 25 -and $count -ne 1; $i++) {
        $count = @(Invoke-RestMethod -Uri "$Base/api/mailboxes/Smoke/emails" -Headers $Auth).Count
        if ($count -ne 1) { Start-Sleep -Milliseconds 200 }
    }
    if ($count -ne 1) { throw "expected 1 captured email, found $count" }
    $id = @(Invoke-RestMethod -Uri "$Base/api/mailboxes/Smoke/emails" -Headers $Auth)[0].id
    $export = Invoke-WebRequest -UseBasicParsing -Uri "$Base/api/emails/$id/export" -Headers $Auth
    if ($export.Content -notmatch 'id="scratchpad-data"') { throw "export is missing the data block" }
    Write-Host "smoke test passed: SMTP capture, health check, web API and export work"
} finally {
    Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
}
