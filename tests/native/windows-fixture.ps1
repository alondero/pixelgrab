param(
    [ValidateSet('desktop', 'receiver', 'save', 'cancel', 'clipboard', 'drag', 'hotkey')]
    [string]$Mode,
    [string]$OutputPath,
    [int]$AppId,
    [int]$X, [int]$Y, [int]$ToX, [int]$ToY
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class FixtureDisplay {
    [DllImport("kernel32.dll")] public static extern uint SetThreadExecutionState(uint flags);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint x, uint y, uint data, UIntPtr extra);
}
'@
# Prevent unattended acceptance from sampling a powered-down display.
[FixtureDisplay]::SetThreadExecutionState([uint32]2147483651) | Out-Null

if ($Mode -eq 'hotkey') {
    [System.Windows.Forms.SendKeys]::SendWait('^+s')
    exit
}

if ($Mode -eq 'clipboard') {
    $bitmap = [System.Windows.Forms.Clipboard]::GetImage()
    if (!$bitmap) { throw 'Clipboard has no bitmap' }
    try { $bitmap.Save($OutputPath, [System.Drawing.Imaging.ImageFormat]::Png) }
    finally { $bitmap.Dispose() }
    exit
}
if ($Mode -eq 'drag') {
    Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class NativeMouse {
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint x, uint y, uint data, UIntPtr extra);
}
'@
    [NativeMouse]::SetCursorPos($X, $Y) | Out-Null
    Start-Sleep -Milliseconds 150
    [NativeMouse]::mouse_event(2, 0, 0, 0, [UIntPtr]::Zero)
    try {
        for ($step = 1; $step -le 40; $step++) {
            [NativeMouse]::SetCursorPos($X + ($ToX - $X) * $step / 40, $Y + ($ToY - $Y) * $step / 40) | Out-Null
            Start-Sleep -Milliseconds 35
        }
        Start-Sleep -Milliseconds 200
    } finally { [NativeMouse]::mouse_event(4, 0, 0, 0, [UIntPtr]::Zero) }
    exit
}
if ($Mode -eq 'save' -or $Mode -eq 'cancel') {
    Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class NativeDialog {
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr hwnd, StringBuilder text, int count);
    delegate bool EnumWindow(IntPtr hwnd, IntPtr data);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumWindow callback, IntPtr data);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent, EnumWindow callback, IntPtr data);
    [DllImport("user32.dll")] static extern int GetDlgCtrlID(IntPtr hwnd);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr SendMessage(IntPtr hwnd, uint message, IntPtr wParam, string text);
    public static IntPtr Filename(IntPtr parent) {
        IntPtr result = IntPtr.Zero;
        EnumChildWindows(parent, (hwnd, data) => {
            var name = new StringBuilder(256); GetClassName(hwnd, name, 256);
            if (IsWindowVisible(hwnd) && name.ToString() == "Edit" && GetDlgCtrlID(hwnd) == 1001) { result = hwnd; return false; }
            return true;
        }, IntPtr.Zero);
        return result;
    }
    public static IntPtr Find(int process) {
        IntPtr result = IntPtr.Zero;
        EnumWindows((hwnd, data) => {
            uint pid; GetWindowThreadProcessId(hwnd, out pid);
            var name = new StringBuilder(256); GetClassName(hwnd, name, 256);
            if (pid == process && IsWindowVisible(hwnd) && name.ToString() == "#32770") { result = hwnd; return false; }
            return true;
        }, IntPtr.Zero);
        return result;
    }
}
'@
    $dialogHandle = [IntPtr]::Zero
    for ($attempt = 0; $attempt -lt 100 -and $dialogHandle -eq [IntPtr]::Zero; $attempt++) {
        $dialogHandle = [NativeDialog]::Find($AppId)
        if ($dialogHandle -eq [IntPtr]::Zero) { Start-Sleep -Milliseconds 100 }
    }
    if ($dialogHandle -eq [IntPtr]::Zero) { throw 'Native Save As dialog did not appear' }
    if ([NativeDialog]::GetForegroundWindow() -ne $dialogHandle) { throw 'Save As dialog is not the foreground window' }
    if ($Mode -eq 'save') {
        $editHandle = [IntPtr]::Zero
        for ($attempt = 0; $attempt -lt 100 -and $editHandle -eq [IntPtr]::Zero; $attempt++) {
            $editHandle = [NativeDialog]::Filename($dialogHandle)
            if ($editHandle -eq [IntPtr]::Zero) { Start-Sleep -Milliseconds 100 }
        }
        if ($editHandle -eq [IntPtr]::Zero) { throw 'Native filename edit did not become ready' }
        [NativeDialog]::SendMessage($editHandle, 12, [IntPtr]::Zero, $OutputPath) | Out-Null
        [System.Windows.Forms.SendKeys]::SendWait('%n')
        [System.Windows.Forms.SendKeys]::SendWait('{ENTER}')
    } else {
        [System.Windows.Forms.SendKeys]::SendWait('{ESC}')
    }
    for ($attempt = 0; $attempt -lt 100 -and [NativeDialog]::IsWindowVisible($dialogHandle); $attempt++) {
        Start-Sleep -Milliseconds 100
    }
    if ([NativeDialog]::IsWindowVisible($dialogHandle)) { throw 'Native Save As dialog did not close' }
    exit
}

$form = New-Object System.Windows.Forms.Form
$form.StartPosition = 'Manual'
$form.Text = 'PixelGrab acceptance fixture'
if ($Mode -eq 'desktop') {
    [FixtureDisplay]::mouse_event(1, 0, 0, 0, [UIntPtr]::Zero)
    $form.FormBorderStyle = 'None'
    $form.Bounds = [System.Windows.Forms.SystemInformation]::VirtualScreen
    $form.BackColor = [System.Drawing.Color]::Coral
    $form.TopMost = $true
    # Auto-close is a backstop: a failed test must not leave a desktop cover.
    $timer = New-Object System.Windows.Forms.Timer
    $timer.Interval = 15000
    $timer.Add_Tick({
        if ($AppId -gt 0) { Stop-Process -Id $AppId -ErrorAction SilentlyContinue }
        $form.Close()
    })
    $timer.Start()
} else {
    $form.Bounds = New-Object System.Drawing.Rectangle(80, 80, 520, 360)
    $form.BackColor = [System.Drawing.Color]::FromArgb(30, 55, 90)
    $form.AllowDrop = $true
    $form.Add_DragEnter({
        param($sender, $eventArgs)
        if ($eventArgs.Data.GetDataPresent([System.Windows.Forms.DataFormats]::FileDrop)) {
            $eventArgs.Effect = [System.Windows.Forms.DragDropEffects]::Copy
        }
    })
    $form.Add_DragDrop({
        param($sender, $eventArgs)
        $paths = $eventArgs.Data.GetData([System.Windows.Forms.DataFormats]::FileDrop)
        [System.IO.File]::WriteAllText($OutputPath, ($paths | ConvertTo-Json -Compress))
    })
}
$form.Add_Shown({
    if ($Mode -ne 'desktop') {
        [System.IO.File]::WriteAllText("$OutputPath.ready", 'ready')
        return
    }
    # Let DWM compose the fixture while the WinForms message loop runs.
    $script:sampleAttempts = 0
    $script:readyTimer = New-Object System.Windows.Forms.Timer
    $readyTimer.Interval = 150
    $readyTimer.Add_Tick({
        $script:sampleAttempts++
        $sample = New-Object System.Drawing.Bitmap(1, 1)
        $graphics = [System.Drawing.Graphics]::FromImage($sample)
        $readable = $false
        try {
            $monitor = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
            $graphics.CopyFromScreen($monitor.Left + 32, $monitor.Top + 32, 0, 0, $sample.Size)
            $color = $sample.GetPixel(0, 0)
            $readable = $color.R -eq 255 -and $color.G -eq 127 -and $color.B -eq 80
        } finally { $graphics.Dispose(); $sample.Dispose() }
        if ($readable) {
            $readyTimer.Stop()
            [System.IO.File]::WriteAllText("$OutputPath.ready", 'ready')
        } elseif ($sampleAttempts -ge 20) {
            $readyTimer.Stop()
            [System.IO.File]::WriteAllText("$OutputPath.unavailable", 'Controlled desktop unreadable after repeated samples; check display state before native acceptance.')
            $form.Close()
        }
    })
    $readyTimer.Start()
})
[System.Windows.Forms.Application]::Run($form)


