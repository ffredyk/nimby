using System;
using System.IO;
using System.Reflection;
using System.Threading.Tasks;
using Avalonia.Controls;
using Avalonia.Interactivity;
using Nimby.App.Services;

namespace Nimby.App.Views;

public partial class AboutView : UserControl, IRefreshable
{
    private readonly AppState? _state;

    public AboutView(AppState? state = null)
    {
        _state = state;
        InitializeComponent();

        VersionText.Text = ShortVersion();
        BuildText.Text = $"Build {BuildDate()} · Windows · GPL-3.0-or-later · © 2026 ffredyk";
    }

    /// <summary>"0.9.0" from the assembly, so the About page and the sidebar agree.</summary>
    public static string ShortVersion()
    {
        var version = typeof(AboutView).Assembly.GetName().Version;
        if (version is null)
        {
            return "0.0.0";
        }
        return $"{version.Major}.{version.Minor}.{version.Build}";
    }

    private static string BuildDate()
    {
        try
        {
            var location = typeof(AboutView).Assembly.Location;
            if (!string.IsNullOrEmpty(location) && File.Exists(location))
            {
                return File.GetLastWriteTime(location).ToString("yyyy-MM-dd");
            }
        }
        catch (Exception)
        {
            // Single-file or trimmed builds have no usable path.
        }
        return "unknown";
    }

    public Task RefreshAsync()
    {
        var path = _state?.Config?.ConfigPath;
        ConfigText.Text = string.IsNullOrWhiteSpace(path) ? "" : $"config: {path}";
        return Task.CompletedTask;
    }

    private void OnOpenWebsite(object? sender, RoutedEventArgs e) => Links.Open(Links.Website);

    private void OnOpenRepository(object? sender, RoutedEventArgs e) => Links.Open(Links.Repository);

    private void OnOpenLicence(object? sender, RoutedEventArgs e) => Links.Open(Links.Licence);

    private void OnOpenNotices(object? sender, RoutedEventArgs e) => Links.Open(Links.NoticeFile);
}
