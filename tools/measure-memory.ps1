#requires -Version 7.0
# 按确切进程采样自然工作集，不修改窗口或工作集。
[CmdletBinding()]
param(
    [Parameter(Mandatory)][int]$TargetProcessId,
    [ValidateRange(10,3600)][int]$Seconds = 180,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [long]$TargetWindowHandle = 0,
    [string]$RendererLabel = 'Slint 1.18.1 / winit / software'
)
$ErrorActionPreference = 'Stop'
$taskOutput = [IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $taskOutput) { throw 'Use a new output directory to preserve previous measurements.' }
New-Item -ItemType Directory -Path $taskOutput | Out-Null
$taskProcess = Get-Process -Id $TargetProcessId
$taskExe = $taskProcess.Path
if (-not ('KoDogMetrics' -as [type])) {
    # 只读取窗口状态与尺寸；DPI上下文仅在采样线程内临时切换。
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class KoDogMetrics {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] static extern bool GetClientRect(IntPtr window, out RECT rectangle);
    [DllImport("user32.dll")] static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
    public static RECT Clntrct(IntPtr window) {
        var previous = SetThreadDpiAwarenessContext(new IntPtr(-4));
        if (previous == IntPtr.Zero) throw new InvalidOperationException("DPI measurement context unavailable.");
        try { RECT rectangle; if (!GetClientRect(window, out rectangle)) throw new InvalidOperationException("Window metrics unavailable."); return rectangle; }
        finally { SetThreadDpiAwarenessContext(previous); }
    }
}
'@
}
$taskExplicitWindow = [IntPtr]::new($TargetWindowHandle)
if ($taskExplicitWindow -ne [IntPtr]::Zero) {
    [uint32]$taskOwner = 0
    if ([KoDogMetrics]::GetWindowThreadProcessId($taskExplicitWindow, [ref]$taskOwner) -eq 0 -or $taskOwner -ne $TargetProcessId) { throw 'Target window does not belong to the measured process.' }
}
$taskPhaseFile = Join-Path $taskOutput 'phase.txt'
'warmup' | Set-Content -LiteralPath $taskPhaseFile -Encoding utf8
$taskRows = [Collections.Generic.List[object]]::new()
$taskWatch = [Diagnostics.Stopwatch]::StartNew()
$taskMeta = [ordered]@{
    ProcessId = $TargetProcessId
    Executable = $taskExe
    Sha256 = (Get-FileHash -LiteralPath $taskExe -Algorithm SHA256).Hash
    GitRevision = (& git rev-parse HEAD)
    StartedAt = $taskProcess.StartTime.ToString('o')
    PowerShell = $PSVersionTable.PSVersion.ToString()
    OS = Get-CimInstance Win32_OperatingSystem | Select-Object Caption,Version,BuildNumber,TotalVisibleMemorySize
    GPU = Get-CimInstance Win32_VideoController | Select-Object Name,DriverVersion
    Renderer = $RendererLabel
    WorkingSetLimitBytes = 60000000
    PrivateWorkingSetSource = 'Win32_PerfFormattedData_PerfProc_Process.WorkingSetPrivate, exact IDProcess'
    ForcedWorkingSetTrim = $false
    ExplicitWindowHandle = $TargetWindowHandle
}
$taskMeta | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $taskOutput 'environment.json') -Encoding utf8
try {
    while ($taskWatch.Elapsed.TotalSeconds -lt $Seconds) {
        $taskProcess.Refresh()
        if ($taskProcess.HasExited) { break }
        $taskPhase = ([string](Get-Content -LiteralPath $taskPhaseFile -Raw)).Trim()
        $taskPrivate = @(Get-CimInstance Win32_PerfFormattedData_PerfProc_Process -Filter "IDProcess=$TargetProcessId")
        if ($taskPrivate.Count -gt 1) { throw 'Ambiguous process performance instance.' }
        $taskWindow = if ($taskExplicitWindow -ne [IntPtr]::Zero) { $taskExplicitWindow } else { $taskProcess.MainWindowHandle }
        $taskRect = if ($taskWindow -ne [IntPtr]::Zero) { [KoDogMetrics]::Clntrct($taskWindow) } else { $null }
        $taskRows.Add([pscustomobject]@{
            Timestamp = [DateTimeOffset]::Now.ToString('o')
            ElapsedSeconds = [Math]::Round($taskWatch.Elapsed.TotalSeconds, 3)
            Phase = $taskPhase
            PhaseStable = $taskPhase.Length -gt 0 -and $taskPhase -eq ([string](Get-Content -LiteralPath $taskPhaseFile -Raw)).Trim()
            ProcessId = $TargetProcessId
            WorkingSetBytes = $taskProcess.WorkingSet64
            PrivateWorkingSetBytes = if ($taskPrivate.Count) { $taskPrivate[0].WorkingSetPrivate } else { $null }
            PrivateCommitBytes = $taskProcess.PrivateMemorySize64
            LifetimePeakWorkingSetBytes = $taskProcess.PeakWorkingSet64
            CpuSeconds = $taskProcess.TotalProcessorTime.TotalSeconds
            Threads = $taskProcess.Threads.Count
            Handles = $taskProcess.HandleCount
            WindowHandle = $taskWindow.ToInt64()
            Dpi = if ($taskRect) { [KoDogMetrics]::GetDpiForWindow($taskWindow) } else { $null }
            ClientWidthPixels = if ($taskRect) { $taskRect.Right - $taskRect.Left } else { $null }
            ClientHeightPixels = if ($taskRect) { $taskRect.Bottom - $taskRect.Top } else { $null }
            Foreground = $taskWindow -ne [IntPtr]::Zero -and [KoDogMetrics]::GetForegroundWindow() -eq $taskWindow
            Minimized = $taskWindow -ne [IntPtr]::Zero -and [KoDogMetrics]::IsIconic($taskWindow)
            Visible = $taskWindow -ne [IntPtr]::Zero -and [KoDogMetrics]::IsWindowVisible($taskWindow)
        })
        Start-Sleep -Milliseconds 1000
    }
} finally {
    $taskRows | Export-Csv -LiteralPath (Join-Path $taskOutput 'samples.csv') -NoTypeInformation -Encoding utf8
}
$taskSummary = $taskRows | Where-Object PhaseStable | Group-Object Phase | ForEach-Object {
    [pscustomobject]@{
        Phase = $_.Name
        Samples = $_.Count
        WorkingSetMeanMB = [Math]::Round(($_.Group | Measure-Object WorkingSetBytes -Average).Average / 1000000, 3)
        WorkingSetMaxMB = [Math]::Round(($_.Group | Measure-Object WorkingSetBytes -Maximum).Maximum / 1000000, 3)
        PrivateWorkingSetMeanMB = if ($null -ne ($_.Group | Measure-Object PrivateWorkingSetBytes -Average).Average) { [Math]::Round(($_.Group | Measure-Object PrivateWorkingSetBytes -Average).Average / 1000000, 3) } else { $null }
        PrivateCommitMeanMB = [Math]::Round(($_.Group | Measure-Object PrivateCommitBytes -Average).Average / 1000000, 3)
    }
}
$taskSummary | Export-Csv -LiteralPath (Join-Path $taskOutput 'summary.csv') -NoTypeInformation -Encoding utf8
$taskSummary | Format-Table -AutoSize
