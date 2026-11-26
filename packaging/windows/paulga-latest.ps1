<#
.SYNOPSIS
  Write dist/release/latest.json for a packaged PAULGA Photo installer.

.DESCRIPTION
  PAULGA Studio reads this file from the newest GitHub release to offer the download and the
  update: {version, url, sha256, size, publishedAt}. `url` is the installer's address on the
  release `paulga-v<version>` of -Repository.
#>
param(
  [Parameter(Mandatory)] [string] $Version,
  [Parameter(Mandatory)] [string] $Repository,
  [string] $Dist
)
$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
if (-not $Dist) { $Dist = Join-Path $Root 'dist\release' }

$msi = Get-Item -LiteralPath (Join-Path $Dist "photocraft-$Version-windows-x64.msi")
$latest = [ordered]@{
  version     = $Version
  url         = "https://github.com/$Repository/releases/download/paulga-v$Version/$($msi.Name)"
  sha256      = (Get-FileHash -Algorithm SHA256 -LiteralPath $msi.FullName).Hash.ToLowerInvariant()
  size        = $msi.Length
  publishedAt = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
}
$path = Join-Path $Dist 'latest.json'
$latest | ConvertTo-Json | Set-Content -LiteralPath $path -Encoding utf8NoBOM
Get-Content -LiteralPath $path
