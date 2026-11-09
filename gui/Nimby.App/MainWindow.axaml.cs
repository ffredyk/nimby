using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Avalonia.Controls;
using Avalonia.Interactivity;
using Nimby.App.Models;
using Nimby.App.Services;
using Nimby.App.Views;

namespace Nimby.App;

public partial class MainWindow : Window
{
    private readonly Dictionary<string, Control> _pages = new();
    private readonly Dictionary<string, Func<Task>> _refreshers = new();
    private AppState? _state;

    public MainWindow()
    {
        InitializeComponent();
        Opened += async (_, _) => await InitializeAsync();
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

        _state.ConfigChanged += OnConfigChanged;

        CliText.Text = $"cli: {cli.Executable}";
        VersionText.Text = $"config: {config?.ConfigPath}";
        FooterText.Text = config is null
            ? "Configuration unavailable."
            : $"workspace {config.WorkspacePath} · {(config.SharedEnabled ? "shared store on" : "shared store off")}";

        NavList.SelectedIndex = 0;

        if (Environment.GetEnvironmentVariable("NIMBY_SMOKE") is { } smoke && smoke.Length > 0)
        {
            await RunSmokeTestAsync(save: smoke.Equals("save", StringComparison.OrdinalIgnoreCase));
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
            await RefreshPageAsync(tag);
        }
        catch (Exception ex)
        {
            FooterText.Text = ex.Message;
        }
    }
}
