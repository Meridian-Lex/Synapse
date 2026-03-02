#Requires -Version 5.1
$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$PluginDir = Join-Path $env:USERPROFILE ".claude\plugins\synapse"

Write-Host "Building synapse-mcp..."
Set-Location $ScriptDir
npm install
npm run build

Write-Host "Installing plugin to $PluginDir ..."
$PluginsParent = Split-Path -Parent $PluginDir
if (-not (Test-Path $PluginsParent)) {
    New-Item -ItemType Directory -Path $PluginsParent | Out-Null
}

if (Test-Path $PluginDir) {
    $item = Get-Item $PluginDir
    if ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) {
        Write-Host "Removing existing junction at $PluginDir"
        Remove-Item $PluginDir -Force
    } else {
        Write-Error "ERROR: $PluginDir exists and is not a junction. Remove it manually before installing."
        exit 1
    }
}

cmd /c "mklink /J `"$PluginDir`" `"$ScriptDir`"" | Out-Null
Write-Host "Done. Plugin installed at $PluginDir -> $ScriptDir"
Write-Host ""
Write-Host "Required environment variables:"
Write-Host "  SYNAPSE_AGENT   -- your agent name"
Write-Host "  SYNAPSE_SECRET  -- your agent secret"
Write-Host "  SYNAPSE_HOST    -- broker address (default: localhost:7777)"
Write-Host "  SYNAPSE_CA      -- CA cert path (default: /etc/synapse/ca.pem)"
Write-Host "  SYNAPSE_CLI     -- path to synapse binary (default: synapse in PATH)"
