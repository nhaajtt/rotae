# Rotae installer for Windows 10 and 11 (x64).
#
#   powershell -c "irm https://rotae.nhaajt.com/install.ps1 | iex"
#
# What it does: asks GitHub for the newest Rotae release, downloads the installer from
# that release, checks its SHA-256 against the one GitHub publishes, then installs it quietly
# for your user only (no admin). Run it again to update.

& {
  $ErrorActionPreference = 'Stop'
  $ProgressPreference = 'SilentlyContinue'
  $repo = 'nhaajtt/rotae'
  $file = $null

  try {
    if (-not [Environment]::Is64BitOperatingSystem) { throw 'Rotae needs 64-bit Windows.' }
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

    Write-Host 'Rotae: looking up the newest release...'
    $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$repo/releases/latest" -Headers @{ 'User-Agent' = 'rotae-install' }
    $asset = $release.assets | Where-Object { $_.name -match '_x64-setup\.exe$' } | Select-Object -First 1
    if (-not $asset) { throw 'No Windows installer found in the newest release.' }

    $url = [string]$asset.browser_download_url
    if (-not $url.StartsWith("https://github.com/$repo/releases/download/")) { throw "Unexpected download address: $url" }
    if ([string]$asset.digest -notmatch '^sha256:([0-9a-fA-F]{64})$') { throw 'The release does not publish a SHA-256, so the download cannot be checked.' }
    $expected = $Matches[1].ToLower()

    $file = Join-Path ([IO.Path]::GetTempPath()) $asset.name
    Write-Host ("Rotae: downloading {0} ({1:N0} MB)..." -f $release.tag_name, ($asset.size / 1MB))
    Invoke-WebRequest -Uri $url -OutFile $file -UseBasicParsing

    $actual = (Get-FileHash -Path $file -Algorithm SHA256).Hash.ToLower()
    if ($actual -ne $expected) { throw "SHA-256 does not match. Expected $expected, got $actual. Nothing was installed." }
    Write-Host 'Rotae: SHA-256 matches.'

    Write-Host 'Rotae: installing...'
    $setup = Start-Process -FilePath $file -ArgumentList '/S' -Wait -PassThru
    if ($setup.ExitCode -ne 0) { throw "The installer stopped with code $($setup.ExitCode)." }

    $where = (Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Rotae' -ErrorAction SilentlyContinue).InstallLocation
    if (-not $where) { $where = Join-Path $env:LOCALAPPDATA 'Rotae' }
    $app = Join-Path ([string]$where).Trim('"') 'Rotae.exe'
    if (Test-Path $app) { Start-Process -FilePath $app }
    Write-Host "Rotae $($release.tag_name) is installed. It lives in the tray by the clock: drag a file and hold Shift." -ForegroundColor Green
  }
  catch {
    Write-Host "Rotae was not installed: $($_.Exception.Message)" -ForegroundColor Red
  }
  finally {
    if ($file -and (Test-Path $file)) { Remove-Item $file -Force -ErrorAction SilentlyContinue }
  }
}
