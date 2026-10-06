#requires -Version 5.1
<#
.SYNOPSIS
Samples working-set memory and average CPU of Nivra and the official Discord client.
.DESCRIPTION
Open both clients and leave them in the SAME scenario, then run this script. It
never starts, stops or inspects either client: it only reads the working set and
processor time of processes named Nivra* (any packaged build) and Discord, once
per interval, for the requested duration, and prints a table.

Run one scenario per invocation, for example:
  - idle on one text channel;
  - a voice call with the window in the foreground;
  - a voice call while screen sharing at 60 FPS.
CPU percentages are shown both as a share of the whole CPU (Task Manager style)
and as a share of one logical core. Nothing here is a guarantee: report the
scenario, the date and the Windows version next to any number.
.EXAMPLE
powershell -NoProfile -File scripts/measure.ps1 -Seconds 60 -Scenario "idle on one text channel"
#>
[CmdletBinding()]
param(
    [ValidateRange(10, 600)] [int] $Seconds = 60,
    [ValidateRange(200, 5000)] [int] $IntervalMilliseconds = 1000,
    [string] $Scenario = 'describe the shared scenario here'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Get-NivraProcesses {
    @(Get-Process -Name 'Nivra*' -ErrorAction SilentlyContinue)
}
function Get-DiscordProcesses {
    @(Get-Process -Name 'Discord' -ErrorAction SilentlyContinue)
}
function Get-CpuSeconds {
    param([object[]] $Processes)
    $total = 0.0
    foreach ($process in $Processes) {
        try {
            $process.Refresh()
            $total += $process.TotalProcessorTime.TotalSeconds
        } catch {
            # The process exited between enumeration and refresh; its last reading
            # was already accounted for by the previous sample.
        }
    }
    return $total
}
function Get-WorkingSetBytes {
    param([object[]] $Processes)
    $total = 0L
    foreach ($process in $Processes) {
        try {
            $process.Refresh()
            $total += $process.WorkingSet64
        } catch {
            # Same as above: a process that exited contributes nothing.
        }
    }
    return $total
}

$nivra = Get-NivraProcesses
$discord = Get-DiscordProcesses
if ($nivra.Count -eq 0 -and $discord.Count -eq 0) {
    throw 'Open Nivra and the official Discord client in the same scenario before running this measurement.'
}
if ($nivra.Count -eq 0) { Write-Warning 'No Nivra* process found; its row will be empty.' }
if ($discord.Count -eq 0) { Write-Warning 'No Discord process found; its row will be empty.' }

$logical = [Math]::Max(1, [Environment]::ProcessorCount)
$nivraCpu = 0.0
$discordCpu = 0.0
$nivraPrevious = Get-CpuSeconds (Get-NivraProcesses)
$discordPrevious = Get-CpuSeconds (Get-DiscordProcesses)
$nivraWorkingSetSum = 0L
$nivraWorkingSetPeak = 0L
$nivraSamples = 0
$discordWorkingSetSum = 0L
$discordWorkingSetPeak = 0L
$discordSamples = 0

$clock = [Diagnostics.Stopwatch]::StartNew()
while ($clock.Elapsed.TotalSeconds -lt $Seconds) {
    Start-Sleep -Milliseconds $IntervalMilliseconds
    $nivraNow = Get-CpuSeconds (Get-NivraProcesses)
    $discordNow = Get-CpuSeconds (Get-DiscordProcesses)
    # Count only forward movement, so a leaked or restarted helper cannot cancel
    # real CPU time and a closed client cannot produce a negative sample.
    $nivraCpu += [Math]::Max(0.0, $nivraNow - $nivraPrevious)
    $discordCpu += [Math]::Max(0.0, $discordNow - $discordPrevious)
    $nivraPrevious = $nivraNow
    $discordPrevious = $discordNow
    $nivraWorkingSet = Get-WorkingSetBytes (Get-NivraProcesses)
    $discordWorkingSet = Get-WorkingSetBytes (Get-DiscordProcesses)
    if ($nivraWorkingSet -gt 0) {
        $nivraWorkingSetSum += $nivraWorkingSet
        $nivraWorkingSetPeak = [Math]::Max($nivraWorkingSetPeak, $nivraWorkingSet)
        $nivraSamples++
    }
    if ($discordWorkingSet -gt 0) {
        $discordWorkingSetSum += $discordWorkingSet
        $discordWorkingSetPeak = [Math]::Max($discordWorkingSetPeak, $discordWorkingSet)
        $discordSamples++
    }
}
$clock.Stop()

$elapsed = [Math]::Max(0.001, $clock.Elapsed.TotalSeconds)
function Get-AverageMegabytes {
    param([long] $Sum, [int] $Samples)
    if ($Samples -eq 0) { return $null }
    return [Math]::Round($Sum / $Samples / 1MB, 1)
}
function Get-AverageCpuPercent {
    param([double] $CpuSeconds, [int] $Cores, [bool] $WholeCpu)
    if ($CpuSeconds -le 0.0) { return 0.0 }
    $divisor = if ($WholeCpu) { $elapsed * $Cores } else { $elapsed }
    return [Math]::Round(100.0 * $CpuSeconds / $divisor, 1)
}

$os = try {
    $system = Get-CimInstance -ClassName Win32_OperatingSystem -ErrorAction Stop
    "$($system.Caption) $($system.Version)"
} catch {
    [Environment]::OSVersion.VersionString
}
$rows = @(
    [pscustomobject]@{
        Client     = 'Nivra'
        Processes  = (Get-NivraProcesses).Count
        'Working set avg (MB)'  = Get-AverageMegabytes $nivraWorkingSetSum $nivraSamples
        'Working set peak (MB)' = Get-AverageMegabytes $nivraWorkingSetPeak $(if ($nivraSamples -gt 0) { 1 } else { 0 })
        'CPU avg (% total)'     = Get-AverageCpuPercent $nivraCpu $logical $true
        'CPU avg (% one core)'  = Get-AverageCpuPercent $nivraCpu $logical $false
    }
    [pscustomobject]@{
        Client     = 'Discord'
        Processes  = (Get-DiscordProcesses).Count
        'Working set avg (MB)'  = Get-AverageMegabytes $discordWorkingSetSum $discordSamples
        'Working set peak (MB)' = Get-AverageMegabytes $discordWorkingSetPeak $(if ($discordSamples -gt 0) { 1 } else { 0 })
        'CPU avg (% total)'     = Get-AverageCpuPercent $discordCpu $logical $true
        'CPU avg (% one core)'  = Get-AverageCpuPercent $discordCpu $logical $false
    }
)

Write-Host ''
Write-Host "Scenario : $Scenario"
Write-Host "Measured : $((Get-Date).ToString('yyyy-MM-dd HH:mm')) - $os - $logical logical processors"
Write-Host "Window   : $([Math]::Round($elapsed, 1)) s ($IntervalMilliseconds ms interval)"
Write-Host ''
$rows | Format-Table -AutoSize
Write-Host 'Report these numbers with the scenario, date and Windows version. No account data,'
Write-Host 'message text or identifiers are read by this script.'
