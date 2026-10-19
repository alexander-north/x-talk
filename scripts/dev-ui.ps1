# Native UI iteration with persistent, isolated data and offline providers.
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$saved = @{}
foreach ($name in 'PATH', 'XTALK_TEST_PROVIDERS', 'XTALK_TEST_DATA_DIR', 'XTALK_TEST_UPDATE_URL') {
    $saved[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}
Push-Location $root
try {
    # Optional project-local runtime; otherwise use the installed Node 24+ runtime.
    $localNode = Join-Path $root '.tmp/tools/node-v24.11.1-win-x64'
    if (Test-Path (Join-Path $localNode 'node.exe')) { $env:PATH = "$localNode;$env:PATH" }
    $version = & node --version
    if ($LASTEXITCODE -ne 0 -or [version]$version.TrimStart('v') -lt [version]'24.11.1') {
        throw 'Node 24.11.1 or newer is required. See README build prerequisites.'
    }
    if (!(Test-Path 'node_modules/.bin/tauri.cmd')) { throw 'Run npm ci first.' }
    & node scripts/verify-media.mjs
    if ($LASTEXITCODE -ne 0) { throw 'Bundled media verification failed.' }
    $env:XTALK_TEST_PROVIDERS = '1'
    $env:XTALK_TEST_DATA_DIR = Join-Path $root '.tmp/ui-dev/data'
    $env:XTALK_TEST_UPDATE_URL = $null
    New-Item -ItemType Directory -Force $env:XTALK_TEST_DATA_DIR | Out-Null
    & node_modules/.bin/tauri.cmd dev --features desktop,test-providers --config src-tauri/tauri.ui-dev.conf.json
    $result = $LASTEXITCODE
} finally {
    foreach ($name in $saved.Keys) {
        [Environment]::SetEnvironmentVariable($name, $saved[$name], 'Process')
    }
    Pop-Location
}
exit $result
