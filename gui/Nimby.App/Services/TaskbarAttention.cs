using System;
using System.Runtime.InteropServices;
using Avalonia.Controls;

namespace Nimby.App.Services;

/// <summary>
/// Gets the user's attention when an install needs a decision. Flashing the
/// taskbar button is the only way to reach someone who switched to another
/// window, so it is used before activating the window (which steals focus).
/// </summary>
public static class TaskbarAttention
{
    private const uint FLASHW_STOP = 0;
    private const uint FLASHW_CAPTION = 0x00000001;
    private const uint FLASHW_TRAY = 0x00000002;
    private const uint FLASHW_TIMERNOFG = 0x0000000C;

    [StructLayout(LayoutKind.Sequential)]
    private struct FLASHWINFO
    {
        public uint cbSize;
        public IntPtr hwnd;
        public uint dwFlags;
        public uint uCount;
        public uint dwTimeout;
    }

    [DllImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool FlashWindowEx(ref FLASHWINFO pwfi);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool SetForegroundWindow(IntPtr hWnd);

    /// <summary>
    /// Flash the window's taskbar button until it gets focus. Falls back to
    /// activating the window on platforms without the Win32 API.
    /// </summary>
    public static void Request(Window? window, bool alsoFocus = false)
    {
        if (window is null)
        {
            return;
        }
        var handle = TryGetHandle(window);
        if (handle != IntPtr.Zero)
        {
            try
            {
                var info = new FLASHWINFO
                {
                    hwnd = handle,
                    dwFlags = FLASHW_CAPTION | FLASHW_TRAY | FLASHW_TIMERNOFG,
                    uCount = uint.MaxValue,
                    dwTimeout = 0,
                };
                info.cbSize = (uint)Marshal.SizeOf(info);
                if (FlashWindowEx(ref info))
                {
                    if (alsoFocus)
                    {
                        SetForegroundWindow(handle);
                    }
                    return;
                }
            }
            catch (DllNotFoundException)
            {
                // Not Windows: fall through.
            }
            catch (EntryPointNotFoundException)
            {
                // Old Windows: fall through.
            }
        }

        try
        {
            window.Activate();
        }
        catch (Exception)
        {
            // Activating is best effort.
        }
    }

    /// <summary>Stop flashing, for example once the user answered.</summary>
    public static void Cancel(Window? window)
    {
        if (window is null)
        {
            return;
        }
        var handle = TryGetHandle(window);
        if (handle == IntPtr.Zero)
        {
            return;
        }
        try
        {
            var info = new FLASHWINFO
            {
                hwnd = handle,
                dwFlags = FLASHW_STOP,
            };
            info.cbSize = (uint)Marshal.SizeOf(info);
            FlashWindowEx(ref info);
        }
        catch (Exception)
        {
            // Nothing to stop.
        }
    }

    private static IntPtr TryGetHandle(Window window)
    {
        try
        {
            var platform = window.TryGetPlatformHandle();
            return platform?.Handle ?? IntPtr.Zero;
        }
        catch (Exception)
        {
            return IntPtr.Zero;
        }
    }
}
