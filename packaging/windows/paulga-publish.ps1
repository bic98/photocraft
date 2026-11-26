<#
.SYNOPSIS
  Publish a packaged PAULGA Photo as the GitHub release `paulga-v<version>` of -Repository.

.DESCRIPTION
  Uploads the installer, the portable ZIP and latest.json to a draft release and publishes it
  only when all three are on it, so PAULGA Studio never reads a latest.json whose installer is
  missing. Needs the GitHub CLI, signed in (or GH_TOKEN) with write access to the repository.
  Run packaging/windows/package.ps1 and paulga-latest.ps1 first.
#>
param(
  [Parameter(Mandatory)] [string] $Version,
  [Parameter(Mandatory)] [string] $Repository,
  [string] $Target,
  [string] $Dist
)
$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
if (-not $Dist) { $Dist = Join-Path $Root 'dist\release' }
if (-not $Target) { $Target = (git -C $Root rev-parse HEAD) }

$tag = "paulga-v$Version"
$files = @(
  (Join-Path $Dist "photocraft-$Version-windows-x64.msi"),
  (Join-Path $Dist "photocraft-$Version-windows-x64-portable.zip"),
  (Join-Path $Dist 'latest.json')
)
foreach ($file in $files) { if (-not (Test-Path -LiteralPath $file)) { throw "missing $file" } }

gh release view $tag --repo $Repository *> $null
if ($LASTEXITCODE -eq 0) {
  gh release upload $tag @files --repo $Repository --clobber
  if ($LASTEXITCODE -ne 0) { throw "upload to $tag failed" }
} else {
  gh release create $tag @files --repo $Repository --target $Target --draft --title "PAULGA Photo $Version" --notes "PAULGA Photo $Version for Windows (x64)."
  if ($LASTEXITCODE -ne 0) { throw "creating $tag failed" }
}
gh release edit $tag --repo $Repository --draft=false --latest
if ($LASTEXITCODE -ne 0) { throw "publishing $tag failed" }
gh release view $tag --repo $Repository --json url,assets --jq '.url, (.assets[] | "  " + .name)'
