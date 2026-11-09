using System;
using System.Diagnostics;

namespace Nimby.App.Services;

/// <summary>
/// Opens external links in the user's browser. Kept in one place so the About
/// page and the sidebar credit share the same URLs.
/// </summary>
public static class Links
{
    public const string Website = "https://nimby.ffredyk.cz";
    public const string Repository = "https://github.com/ffredyk/nimby";
    public const string Licence = "https://www.gnu.org/licenses/gpl-3.0.html";
    public const string NoticeFile = Repository + "/blob/main/THIRD-PARTY-NOTICES.md";

    /// <summary>Best-effort shell open; a missing browser must never crash the UI.</summary>
    public static void Open(string url)
    {
        if (string.IsNullOrWhiteSpace(url))
        {
            return;
        }
        try
        {
            Process.Start(new ProcessStartInfo
            {
                FileName = url,
                UseShellExecute = true,
            });
        }
        catch (Exception)
        {
            // No shell handler, or the shell is unavailable in this session.
        }
    }
}
