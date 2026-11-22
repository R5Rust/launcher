<#
  release.ps1 - builds the player launcher and (with -Publish) releases it on GitHub as v<Cargo.toml version>.

    pwsh tools\release.ps1 [-Publish] [-Notes 'Line one','Line two']

  The exe is built with the C runtime linked in (a clean PC may lack the VC++ redist) and with this PC's paths remapped
  out, checked for both, and uploaded as R5Rust-Launcher.exe (the download link and the self-updater use that name).
  Installed launchers offer the update on their next start.
#>
param([switch]$Publish, [string[]]$Notes = @(), [string]$TargetDir = 'D:\R5Rust\_build\launcher-release')
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Push-Location $root
try {
  $version = (Select-String Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value
  $env:CARGO_TARGET_DIR = $TargetDir
  $env:RUSTFLAGS = "-C target-feature=+crt-static --remap-path-prefix=$env:USERPROFILE=~ --remap-path-prefix=C:\Code=src"
  cargo build --release
  if ($LASTEXITCODE) { throw 'build failed' }
  Remove-Item Env:RUSTFLAGS, Env:CARGO_TARGET_DIR
  $dist = Join-Path $root 'dist'
  New-Item -ItemType Directory -Force $dist | Out-Null
  $exe = Join-Path $dist 'R5Rust-Launcher.exe'
  Copy-Item (Join-Path $TargetDir 'release\r5rust-launcher.exe') $exe -Force

  # checks: no user folder baked in, no VC++ runtime DLL needed
  if (Select-String -LiteralPath $exe -Pattern ([regex]::Escape($env:USERPROFILE)) -Encoding ascii -List) { throw 'the exe has the user folder baked in' }
  $vc = 'C:\Program Files\Microsoft Visual Studio\18\Community\VC\Auxiliary\Build\vcvars64.bat'
  $deps = cmd /c "`"$vc`" >nul 2>&1 && dumpbin /dependents `"$exe`"" | Select-String '\.dll' | ForEach-Object { $_.Line.Trim() }
  $bad = $deps | Where-Object { $_ -match '^(vcruntime|msvcp|api-ms-win-crt)' }
  if ($bad) { throw "needs the VC++ runtime: $($bad -join ', ')" }
  '{0}  v{1}  {2:N1} MB  imports: {3}' -f $exe, $version, ((Get-Item $exe).Length / 1MB), ($deps -join ' ')

  if ($Publish) {
    $lines = if ($Notes.Count) { ($Notes | ForEach-Object { "- $_" }) -join "`n" } else { '- Fixes and improvements' }
    $body = "**[Download R5Rust-Launcher.exe](https://github.com/R5Rust/launcher/releases/latest/download/R5Rust-Launcher.exe)**, run it, done: it installs itself and opens. Installed launchers update themselves (bottom left: **Update**).`n`n" +
      "Windows may say `"Windows protected your PC`" (the exe isn't code-signed yet): press **More info > Run anyway**.`n`n### Patch notes`n$lines`n"
    $nf = Join-Path $dist "notes-v$version.md"
    [IO.File]::WriteAllText($nf, $body)
    gh release create "v$version" $exe -R R5Rust/launcher --target main --title "R5Rust Launcher $version" --notes-file $nf
    if ($LASTEXITCODE) { throw 'gh release create failed' }
    Remove-Item $nf
  }
} finally { Pop-Location }
