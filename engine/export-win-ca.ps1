# Exports the Windows trusted-root certificates (which include any corporate /
# AV TLS-intercepting proxy CA) to a PEM bundle, so R's libcurl can verify HTTPS
# to CRAN / Bioconductor / GitHub behind such a proxy.
#   Usage:  powershell -ExecutionPolicy Bypass -File engine\export-win-ca.ps1
#   Then:   $env:CURL_CA_BUNDLE = "<repo>\engine\win-ca-bundle.pem"
$ErrorActionPreference = "Stop"
$out = Join-Path $PSScriptRoot "win-ca-bundle.pem"
$nl = "`n"
$lines = New-Object System.Collections.Generic.List[string]
Get-ChildItem Cert:\LocalMachine\Root, Cert:\CurrentUser\Root | ForEach-Object {
    $b64 = [Convert]::ToBase64String($_.RawData, 'InsertLineBreaks')
    $lines.Add("-----BEGIN CERTIFICATE-----")
    foreach ($l in ($b64 -split "`r?`n")) { $lines.Add($l) }
    $lines.Add("-----END CERTIFICATE-----")
}
Set-Content -Path $out -Value $lines -Encoding ascii
Write-Host "Wrote $out ($($lines.Count) lines)"
