#Requires -Version 5.1
$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$PluginDir = Join-Path $env:USERPROFILE ".claude\plugins\synapse"

Write-Output "Building synapse-mcp..."
Set-Location $ScriptDir
npm ci
if ($LASTEXITCODE -ne 0) { throw "npm ci failed with exit code $LASTEXITCODE" }
npm run build
if ($LASTEXITCODE -ne 0) { throw "npm run build failed with exit code $LASTEXITCODE" }

Write-Output "Installing plugin to $PluginDir ..."
$PluginsParent = Split-Path -Parent $PluginDir
if (-not (Test-Path $PluginsParent)) {
    New-Item -ItemType Directory -Path $PluginsParent | Out-Null
}

if (Test-Path $PluginDir) {
    $item = Get-Item $PluginDir
    if ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) {
        Write-Output "Removing existing junction at $PluginDir"
        Remove-Item $PluginDir -Force
    } else {
        Write-Error "ERROR: $PluginDir exists and is not a junction. Remove it manually before installing."
        exit 1
    }
}

cmd /c "mklink /J `"$PluginDir`" `"$ScriptDir`"" | Out-Null
if ($LASTEXITCODE -ne 0) { throw "mklink failed with exit code $LASTEXITCODE" }
Write-Output "Done. Plugin installed at $PluginDir -> $ScriptDir"
Write-Output ""
Write-Output "Required environment variables:"
Write-Output "  SYNAPSE_AGENT   -- your agent name"
Write-Output "  SYNAPSE_SECRET  -- your agent secret"
Write-Output "  SYNAPSE_HOST    -- broker address (default: localhost:7777)"
Write-Output "  SYNAPSE_CA      -- CA cert path (default: /etc/synapse/ca.pem)"
Write-Output "  SYNAPSE_CLI     -- path to synapse binary (default: synapse in PATH)"
