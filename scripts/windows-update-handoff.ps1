# The Windows self-update, run for real (release.yml, windows-update):
# iw4l.exe hands its start to iw4launcher.exe, which installs what the
# manifest names, replacing itself while it runs, and starts the new game.
#
#   -Test local   an older release's iw4l.exe in this run's folder, updated
#                 from this run's update-test/ through a file:// URL; then
#                 this run's iw4l.exe hands off and finds both up to date.
#   -Test github  an older release as published, updated from the newest
#                 release over HTTPS with no setting at all.
param(
    [Parameter(Mandatory)][ValidateSet('local', 'github')][string]$Test,
    [string]$Zip = 'zip/2010-Rust-Rewrite-Mashup-windows-x64.zip',
    [string]$OldZip = 'old/2010-Rust-Rewrite-Mashup-windows-x64.zip',
    [string]$UpdateTest = 'update-test',
    [string]$LatestManifest = 'latest/manifest.json'
)
$ErrorActionPreference = 'Stop'
$failures = [System.Collections.Generic.List[string]]::new()
function Check([bool]$ok, [string]$what) {
    if ($ok) { Write-Host "ok    $what" } else { Write-Host "FAIL  $what"; $failures.Add($what) }
}

function Expand-Release([string]$zip, [string]$name) {
    $to = Join-Path $env:RUNNER_TEMP $name
    if (Test-Path $to) { Remove-Item -Recurse -Force $to }
    Expand-Archive -Path $zip -DestinationPath $to
    return (Get-ChildItem -Directory $to | Select-Object -First 1).FullName
}

function Get-Short([string]$hash) { return $hash.Substring(0, [math]::Min(12, $hash.Length)) }

function Get-Sha([string]$path) {
    if (-not (Test-Path $path)) { return 'missing' }
    return (Get-FileHash -Algorithm SHA256 $path).Hash.ToLowerInvariant()
}

# Starts iw4l.exe and waits until no process runs from $dir any more: the
# game hands off and exits, the launcher starts the game and exits.
function Invoke-Chain([string]$dir, [string[]]$arguments, [int]$timeoutSec) {
    $tag = "$(Split-Path $dir -Leaf)-$($arguments -join '-' -replace '[^\w-]', '')"
    $out = Join-Path $env:RUNNER_TEMP "$tag.out"
    $err = Join-Path $env:RUNNER_TEMP "$tag.err"
    $in = Join-Path $env:RUNNER_TEMP "$tag.in"
    Set-Content -Path $in -Value '' -NoNewline
    $clock = [Diagnostics.Stopwatch]::StartNew()
    Start-Process -FilePath (Join-Path $dir 'iw4l.exe') -ArgumentList $arguments -WorkingDirectory $dir `
        -RedirectStandardOutput $out -RedirectStandardError $err -RedirectStandardInput $in -NoNewWindow | Out-Null
    $prefix = $dir.TrimEnd('\') + '\'
    $running = @()
    do {
        Start-Sleep -Milliseconds 200
        $running = @(Get-Process -Name iw4l, iw4launcher -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path.StartsWith($prefix, 'OrdinalIgnoreCase') })
    } while ($running.Count -gt 0 -and $clock.Elapsed.TotalSeconds -lt $timeoutSec)
    $seconds = [math]::Round($clock.Elapsed.TotalSeconds, 1)
    $left = @($running | ForEach-Object { "$($_.Id) $($_.Path)" })
    $running | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 500
    $text = (Get-Content -Raw $out -ErrorAction SilentlyContinue) + (Get-Content -Raw $err -ErrorAction SilentlyContinue)
    Write-Host "--- iw4l.exe $($arguments -join ' ') ($seconds s$(if ($left) { ", still running at the timeout: $($left -join ', ')" }))"
    Write-Host $text
    Write-Host '---'
    return [pscustomobject]@{ Text = "$text"; Seconds = $seconds; Left = $left }
}

function Show-Swap([string]$dir, $manifest, [hashtable]$before) {
    $rows = foreach ($file in $manifest.files) {
        [pscustomobject]@{
            file     = $file.path
            before   = Get-Short $before[$file.path]
            after    = Get-Short (Get-Sha (Join-Path $dir $file.path))
            manifest = Get-Short $file.sha256
        }
    }
    $rows | Format-Table -AutoSize | Out-String | Write-Host
    Get-ChildItem $dir -File | Format-Table Name, Length, LastWriteTime -AutoSize | Out-String | Write-Host
    foreach ($file in $manifest.files) {
        Check ((Get-Sha (Join-Path $dir $file.path)) -eq $file.sha256.ToLowerInvariant()) "$($file.path) is the manifest's $($file.sha256.Substring(0, 12))"
    }
    Check (-not (Get-ChildItem $dir -Filter '*.part')) 'no .part left'
}

function Get-Hashes([string]$dir, $manifest) {
    $hashes = @{}
    foreach ($file in $manifest.files) { $hashes[$file.path] = Get-Sha (Join-Path $dir $file.path) }
    return $hashes
}

if ($Test -eq 'local') {
    $manifest = Get-Content -Raw (Join-Path $UpdateTest 'manifest.json') | ConvertFrom-Json
    $dir = Expand-Release $Zip 'local'
    $old = Expand-Release $OldZip 'local-old'
    Copy-Item -Force (Join-Path $old 'iw4l.exe') (Join-Path $dir 'iw4l.exe')
    $env:IW4L_UPDATE_URL = 'file:///' + ((Resolve-Path $UpdateTest).Path -replace '\\', '/')
    Write-Host "IW4L_UPDATE_URL=$env:IW4L_UPDATE_URL"
    $before = Get-Hashes $dir $manifest
    foreach ($file in $manifest.files) {
        Check ($before[$file.path] -ne $file.sha256) "$($file.path) differs from the manifest before the update"
    }

    $run = Invoke-Chain $dir @('--version') 120
    Show-Swap $dir $manifest $before
    Check ($run.Left.Count -eq 0) 'the chain ended'
    Check ($run.Text -match '\[iw4launcher\] installed iw4l\.exe') 'the launcher installed iw4l.exe'
    Check ($run.Text -match '\[iw4launcher\] installed iw4launcher\.exe') 'the launcher installed itself'
    Check ($run.Text -match "(?m)^iw4l $env:GITHUB_SHA protocol=\d+ master_protocol=\d+") "the new iw4l.exe printed --version for $env:GITHUB_SHA"

    # This run's iw4l.exe (a pull request's is built with no update URL)
    # hands off because IW4L_UPDATE_URL is set; nothing is left to install.
    $after = Get-Hashes $dir $manifest
    $run = Invoke-Chain $dir @('--help') 120
    Check ($run.Left.Count -eq 0) 'the second chain ended'
    Check ($run.Text -match '\[iw4launcher\] iw4l\.exe up to date') 'iw4l.exe up to date'
    Check ($run.Text -match '\[iw4launcher\] iw4launcher\.exe up to date') 'iw4launcher.exe up to date'
    Check ($run.Text -match 'dedicated <zone>') 'the game printed its usage'
    Check (((Get-Hashes $dir $manifest) | ConvertTo-Json) -eq ($after | ConvertTo-Json)) 'nothing changed'
}
else {
    $manifest = Get-Content -Raw $LatestManifest | ConvertFrom-Json
    Write-Host "newest release: build $($manifest.build), git $($manifest.git)"
    Remove-Item Env:IW4L_UPDATE_URL, Env:IW4L_UPDATE, Env:IW4L_LAUNCHER -ErrorAction SilentlyContinue
    $dir = Expand-Release $OldZip 'github'
    $before = Get-Hashes $dir $manifest
    $run = Invoke-Chain $dir @('--help') 60
    Show-Swap $dir $manifest $before
    Check ($run.Text -match "\[iw4launcher\] updated to channel=github protocol=\d+ git=$($manifest.git) build=$($manifest.build)") "the launcher updated to build $($manifest.build)"
    Check ($run.Text -match '\[iw4launcher\] starting iw4l\.exe --help') 'the launcher started the new iw4l.exe'
    # A release from before --help came ahead of first-run setup stops at
    # its setup dialog here: that is a started game too.
    $usage = $run.Text -match 'dedicated <zone>'
    $waiting = ($run.Left.Count -gt 0) -and (@($run.Left | Where-Object { $_ -match '\\iw4l\.exe$' }).Count -eq $run.Left.Count)
    Check ($usage -or $waiting) 'the new iw4l.exe ran (printed its usage, or waited in first-run setup)'
    if ($waiting) { Write-Host 'note: the new iw4l.exe was in first-run setup at the timeout' }
}

if ($failures.Count) {
    Write-Host "::error::$($failures.Count) check(s) failed: $($failures -join '; ')"
    exit 1
}
