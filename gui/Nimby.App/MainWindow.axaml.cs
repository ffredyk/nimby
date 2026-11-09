using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Threading.Tasks;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Interactivity;
using Avalonia.Media.Imaging;
using Avalonia.Platform;
using Nimby.App.Models;
using Nimby.App.Services;
using Nimby.App.Views;

namespace Nimby.App;

public partial class MainWindow : Window
{
    private const string ProductIcon = "avares://Nimby/Assets/nimby-256.png";

    private readonly Dictionary<string, Control> _pages = new();
    private readonly Dictionary<string, Func<Task>> _refreshers = new();
    private AppState? _state;
    private bool _capturing;

    public MainWindow()
    {
        InitializeComponent();
        SetWindowIcon();
        VersionText.Text = $"Nimby {ProductVersion()}";
        Opened += async (_, _) => await InitializeAsync();
    }

    /// <summary>"0.9.0" from the assembly version set in the project file.</summary>
    private static string ProductVersion()
    {
        var version = typeof(App).Assembly.GetName().Version;
        return version is null ? "0.0.0" : $"{version.Major}.{version.Minor}.{version.Build}";
    }

    private void SetWindowIcon()
    {
        try
        {
            using var stream = AssetLoader.Open(new Uri(ProductIcon));
            Icon = new WindowIcon(stream);
        }
        catch (Exception)
        {
            // A missing icon must not stop the app from starting.
        }
    }

    private async Task InitializeAsync()
    {
        CliRunner cli;
        try
        {
            cli = CliRunner.Locate();
        }
        catch (Exception ex)
        {
            ShowFatal(ex.Message);
            return;
        }

        _state = new AppState(cli);
        var config = await _state.RefreshConfigAsync();

        RegisterPage("install", new InstallView(_state));
        RegisterPage("repos", new ReposView(_state));
        RegisterPage("config", new ConfigView(_state));
        RegisterPage("params", new ParamsView(_state));
        RegisterPage("stats", new StatsView(_state));
        RegisterPage("launch", new LaunchView(_state));
        RegisterPage("about", new AboutView(_state));

        _state.ConfigChanged += OnConfigChanged;

        FooterText.Text = config is null
            ? "Configuration unavailable."
            : $"workspace {config.WorkspacePath} · {(config.SharedEnabled ? "shared store on" : "shared store off")}";

        NavList.SelectedIndex = 0;

        if (Environment.GetEnvironmentVariable("NIMBY_SMOKE_SHOTS") is { Length: > 0 } shotsDir)
        {
            await CaptureScreensAsync(shotsDir);
        }

        if (Environment.GetEnvironmentVariable("NIMBY_SMOKE") is { } smoke && smoke.Length > 0)
        {
            await RunSmokeTestAsync(save: smoke.Equals("save", StringComparison.OrdinalIgnoreCase));
        }
    }

    /// <summary>
    /// Renders every page and writes a PNG per page. Used to refresh the
    /// screenshots in docs/screenshots without touching the desktop: only this
    /// window's own rendered frame is captured.
    /// </summary>
    private async Task CaptureScreensAsync(string directory)
    {
        var shots = new (string Tag, string File)[]
        {
            ("install", "01-install.png"),
            ("repos", "02-repos.png"),
            ("config", "03-config.png"),
            ("params", "04-parameters.png"),
            ("stats", "05-stats.png"),
            ("launch", "06-launch.png"),
            ("about", "07-about.png"),
        };

        _capturing = true;
        try
        {
            Directory.CreateDirectory(directory);
            var index = 0;
            foreach (var (tag, file) in shots)
            {
                if (!_pages.TryGetValue(tag, out var page))
                {
                    Console.WriteLine($"shot skip {tag}: no page");
                    continue;
                }
                NavList.SelectedIndex = index++;
                PageHost.Content = page;
                await RefreshPageAsync(tag);
                // Two layout passes plus a delay: the pages fetch asynchronously,
                // and the renderer needs one frame with the final layout.
                UpdateLayout();
                page.UpdateLayout();
                await Task.Delay(150);
                UpdateLayout();
                await Task.Delay(50);

                // Avalonia 12 has no TopLevel.CaptureRenderedFrame(), so render
                // this window's own visual tree through the render target API.
                var path = Path.Combine(directory, file);
                if (TryCaptureFrame(path, out var size))
                {
                    Console.WriteLine($"shot {file} {size}");
                }
                else
                {
                    Console.WriteLine($"shot fail {file}: {size}");
                }
            }
        }
        catch (Exception ex)
        {
            Console.WriteLine($"shot fail: {ex}");
        }
        finally
        {
            _capturing = false;
        }

        Environment.Exit(0);
    }

    /// <summary>Renders this window (and only this window) to a PNG.</summary>
    private bool TryCaptureFrame(string path, out string info)
    {
        try
        {
            var bounds = Bounds;
            var scaling = RenderScaling <= 0 ? 1.0 : RenderScaling;
            var width = Math.Max(1, (int)Math.Round(bounds.Width * scaling));
            var height = Math.Max(1, (int)Math.Round(bounds.Height * scaling));
            using var bitmap = new RenderTargetBitmap(
                new PixelSize(width, height),
                new Vector(96 * scaling, 96 * scaling));
            bitmap.Render(this);
            bitmap.Save(path, PngBitmapEncoderOptions.Default);
            info = $"{width}x{height}";
            return true;
        }
        catch (Exception ex)
        {
            info = ex.Message;
            return false;
        }
    }

    /// <summary>
    /// Headless self-check used by CI: build every page, refresh it and report
    /// whether any page threw. Exits non-zero on the first failure.
    /// </summary>
    private async Task RunSmokeTestAsync(bool save)
    {
        var failures = 0;
        foreach (var (tag, refresh) in _refreshers)
        {
            try
            {
                PageHost.Content = _pages[tag];
                await refresh();
                if (_state?.LastError is { } error)
                {
                    failures++;
                    Console.WriteLine($"fail {tag}: {error}");
                }
                else
                {
                    Console.WriteLine($"ok   {tag}");
                }
            }
            catch (Exception ex)
            {
                failures++;
                Console.WriteLine($"fail {tag}: {ex.Message}");
            }
        }

        if (save && _state is not null)
        {
            // Reproduce the "Save settings" button path: run `config set`, which
            // raises ConfigChanged and refreshes the visible page.
            try
            {
                PageHost.Content = _pages["config"];
                var parallel = (_state.Config?.ParallelDownloads ?? 8).ToString();
                var result = await _state.SetConfigAsync(["--parallel", parallel]);
                Console.WriteLine($"save ok={result.Ok} err={result.StdErr.Trim()}");
                foreach (var (tag, refresh) in _refreshers)
                {
                    await refresh();
                    Console.WriteLine($"post-save ok {tag}");
                }
            }
            catch (Exception ex)
            {
                failures++;
                Console.WriteLine($"fail save: {ex}");
            }
        }

        if (save && _state is not null && Environment.GetEnvironmentVariable("NIMBY_SMOKE_INSTALL") == "1")
        {
            // Exercise the real install path, including streaming CLI output into
            // the log panel from a background thread.
            try
            {
                if (_pages["install"] is InstallView install)
                {
                    PageHost.Content = install;
                    await install.RefreshAsync();
                    var smallest = await _state.SnapshotAsync(_state.ActiveRepoUrl);
                    // Cancelling needs a mod that takes long enough to interrupt.
                    var cancelSmoke = Environment.GetEnvironmentVariable("NIMBY_SMOKE_CANCEL") == "1";
                    var pick = cancelSmoke
                        ? smallest?.Mods
                            .Where(m => m.Bytes is > 50_000_000 and < 500_000_000)
                            .OrderByDescending(m => m.Bytes)
                            .FirstOrDefault()
                        : smallest?.Mods
                            .Where(m => m.Bytes < 30_000_000)
                            .OrderBy(m => m.Bytes)
                            .FirstOrDefault();
                    if (pick is null)
                    {
                        Console.WriteLine("install skipped: no small mod in repo");
                    }
                    else
                    {
                        Console.WriteLine($"install pick {pick.Name} ({pick.Size})");
                        // NIMBY_SMOKE_CANCEL=1 exercises the cancel handshake: the
                        // mod is downloaded for real and then stopped, so the
                        // resume path is covered end to end.
                        var installTask = install.InstallModsAsync([pick.Name], noWorkshop: cancelSmoke);
                        // Sample the panel while the install runs: proves the UI
                        // keeps painting (and the bar moves) mid-download.
                        for (var i = 0; i < 8 && !installTask.IsCompleted; i++)
                        {
                            await Task.Delay(400);
                            Console.WriteLine($"progress sample {i}: {install.ProgressSnapshot()}");
                            Console.WriteLine($"row sample {i}: {install.RowSnapshot(pick.Name)}");
                            if (cancelSmoke && i == 2)
                            {
                                install.CancelInstall();
                            }
                        }
                        await installTask;
                        await Task.Delay(300);
                        Console.WriteLine($"install status {install.CurrentStatus}");
                        Console.WriteLine($"install progress {install.ProgressSnapshot()}");
                        Console.WriteLine($"install row {install.RowSnapshot(pick.Name)}");
                        if (cancelSmoke && !install.CurrentStatus.Contains("Cancelled"))
                        {
                            failures++;
                            Console.WriteLine("fail cancel: install was not cancelled");
                        }
                    }
                }
            }
            catch (Exception ex)
            {
                failures++;
                Console.WriteLine($"fail install: {ex}");
            }
        }

        // Report the data the pages were populated with, so an empty (but
        // non-throwing) UI is still visible in CI output.
        var config = _state?.Config;
        Console.WriteLine($"data cli={_state?.Cli.Executable} error={_state?.LastError ?? "(none)"}");
        Console.WriteLine($"data windowIcon={Icon is not null} version={ProductVersion()}");
        Console.WriteLine(
            $"data repos={config?.Repos.Count ?? 0} params={config?.Params.Count ?? 0} " +
            $"shared={config?.SharedEnabled} arma={config?.ArmaValid}");
        if (_state is not null)
        {
            var snapshot = await _state.SnapshotAsync(_state.ActiveRepoUrl);
            Console.WriteLine(
                $"data activeRepo={_state.ActiveRepoUrl} workspace={_state.WorkspacePath} " +
                $"mods={snapshot?.Mods.Count ?? 0} " +
                $"banner={snapshot?.BannerPath is not null} icon={snapshot?.IconPath is not null}");
            var stats = await _state.StatsAsync();
            Console.WriteLine(
                $"data statsRepos={stats?.Repos.Count ?? 0} saved={stats?.SavedText ?? "n/a"} " +
                $"onDisk={stats?.OnDiskText ?? "n/a"} downloaded={stats?.DownloadedText ?? "n/a"} " +
                $"tracked={stats?.TrackedMods ?? 0}/{stats?.TrackedFiles ?? 0}");
        }

        Console.WriteLine(failures == 0 ? "smoke: pass" : $"smoke: {failures} page(s) failed");
        Environment.Exit(failures == 0 ? 0 : 1);
    }

    private void RegisterPage(string tag, Control page)
    {
        _pages[tag] = page;
        if (page is IRefreshable refreshable)
        {
            _refreshers[tag] = refreshable.RefreshAsync;
        }
    }

    private void ShowFatal(string message)
    {
        PageHost.Content = new TextBlock
        {
            Text = message,
            TextWrapping = Avalonia.Media.TextWrapping.Wrap,
            Margin = new Avalonia.Thickness(24),
        };
        NavList.IsEnabled = false;
        FooterText.Text = "Build the CLI with `cargo build --release` or set NIMBY_CLI.";
    }

    private void OnConfigChanged()
    {
        if (_state?.Config is { } config)
        {
            FooterText.Text =
                $"workspace {config.WorkspacePath} · {(config.SharedEnabled ? "shared store on" : "shared store off")}";
        }
        // Pages refresh themselves after their own changes; other pages pick up
        // the new configuration when the user navigates to them.
    }

    private async Task RefreshPageAsync(string tag)
    {
        if (_refreshers.TryGetValue(tag, out var refresh))
        {
            try
            {
                await refresh();
            }
            catch (Exception ex)
            {
                FooterText.Text = ex.Message;
            }
        }
    }

    private async void OnNavChanged(object? sender, SelectionChangedEventArgs e)
    {
        try
        {
            if (NavList.SelectedItem is not ListBoxItem item || item.Tag is not string tag)
            {
                return;
            }
            if (!_pages.TryGetValue(tag, out var page))
            {
                return;
            }
            PageHost.Content = page;
            if (_capturing)
            {
                // Screenshot mode drives navigation itself.
                return;
            }
            await RefreshPageAsync(tag);
        }
        catch (Exception ex)
        {
            FooterText.Text = ex.Message;
        }
    }

    private void OnOpenWebsite(object? sender, RoutedEventArgs e) => Links.Open(Links.Website);

    private void OnOpenRepository(object? sender, RoutedEventArgs e) => Links.Open(Links.Repository);

    private void OnOpenLicence(object? sender, RoutedEventArgs e) => Links.Open(Links.Licence);
}
