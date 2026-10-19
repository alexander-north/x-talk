# Test setup only: select the same installed WebView2 runtime for the app and its driver.
$ErrorActionPreference = 'Stop'
$candidates = @(
  "${env:ProgramFiles(x86)}\Microsoft\EdgeWebView\Application\*\msedgewebview2.exe",
  "$env:ProgramFiles\Microsoft\EdgeWebView\Application\*\msedgewebview2.exe",
  "$env:LOCALAPPDATA\Microsoft\EdgeWebView\Application\*\msedgewebview2.exe"
)
$runtime = Get-ChildItem $candidates -ErrorAction SilentlyContinue |
  Sort-Object { [version]$_.VersionInfo.ProductVersion } -Descending | Select-Object -First 1
if (!$runtime) { throw 'Install the WebView2 Runtime before configuring WebDriver.' }
$version = $runtime.VersionInfo.ProductVersion
if ($version -notmatch '^\d+\.\d+\.\d+\.\d+$') { throw 'Unexpected WebView2 version format' }
$destination = Join-Path $env:USERPROFILE '.cargo\bin'
$archive = Join-Path ([System.IO.Path]::GetTempPath()) 'xtalk-msedgedriver.zip'
Invoke-WebRequest "https://msedgedriver.microsoft.com/$version/edgedriver_win64.zip" -OutFile $archive
Expand-Archive $archive $destination -Force
Remove-Item $archive
$env:WEBVIEW2_BROWSER_EXECUTABLE_FOLDER = $runtime.DirectoryName
if ($env:GITHUB_ENV) {
  "WEBVIEW2_BROWSER_EXECUTABLE_FOLDER=$($runtime.DirectoryName)" | Out-File $env:GITHUB_ENV -Encoding utf8 -Append
}
Write-Output "WebDriver configured for WebView2 $version"
