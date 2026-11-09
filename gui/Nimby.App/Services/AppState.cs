using System;
using System.Collections.Generic;
using System.Threading;
using System.Threading.Tasks;
using Avalonia.Threading;
using Nimby.App.Models;
using Nimby.App.Services;

namespace Nimby.App.Services;

/// <summary>
/// Shared state for every page: the CLI wrapper plus the last config snapshot.
/// Pages call <see cref="SetConfigAsync"/> and re-read the snapshot afterwards.
/// </summary>
public sealed class AppState
{
    private readonly CliRunner _cli;
    private readonly Dictionary<string, RepoSnapshot> _snapshots = new(StringComparer.OrdinalIgnoreCase);

    public AppState(CliRunner cli) => _cli = cli;

    public CliRunner Cli => _cli;

    public ConfigSnapshot? Config { get; private set; }

    /// <summary>Last CLI failure, for diagnostics in the smoke test.</summary>
    public string? LastError { get; private set; }

    public string ActiveRepoUrl
    {
        get
        {
            var config = Config;
            if (config is null)
            {
                return "";
            }
            if (!string.IsNullOrEmpty(config.ActiveRepo))
            {
                return config.ActiveRepo!;
            }
            return config.Repos.Count > 0 ? config.Repos[0].Url : "";
        }
    }

    public string WorkspacePath => !string.IsNullOrEmpty(Config?.ActiveWorkspace)
        ? Config!.ActiveWorkspace
        : Config?.WorkspacePath ?? "";

    /// <summary>Raised whenever the persisted configuration changed.</summary>
    public event Action? ConfigChanged;

    /// <summary>
    /// CLI calls resume on the thread pool, so subscribers must be notified on
    /// the UI thread: they update controls.
    /// </summary>
    private void RaiseConfigChanged()
    {
        var handler = ConfigChanged;
        if (handler is null)
        {
            return;
        }
        if (Dispatcher.UIThread.CheckAccess())
        {
            handler();
        }
        else
        {
            Dispatcher.UIThread.Post(handler);
        }
    }

    public async Task<ConfigSnapshot?> RefreshConfigAsync(CancellationToken token = default)
    {
        try
        {
            using var document = await _cli.RunJsonAsync(["config"], token).ConfigureAwait(false);
            Config = ConfigSnapshot.FromJson(document);
            LastError = null;
            return Config;
        }
        catch (Exception ex)
        {
            LastError = ex.Message;
            return Config;
        }
    }

    /// <summary>Run `nimby config set ...` and refresh the snapshot.</summary>
    public async Task<CliRunner.RunResult> SetConfigAsync(
        IEnumerable<string> args,
        CancellationToken token = default)
    {
        var list = new List<string> { "config", "set" };
        list.AddRange(args);
        var result = await _cli.RunAsync(list, token).ConfigureAwait(false);
        await RefreshConfigAsync(token).ConfigureAwait(false);
        RaiseConfigChanged();
        return result;
    }

    public async Task<CliRunner.RunResult> RepoAsync(
        IEnumerable<string> args,
        CancellationToken token = default)
    {
        var list = new List<string> { "repo" };
        list.AddRange(args);
        var result = await _cli.RunAsync(list, token).ConfigureAwait(false);
        await RefreshConfigAsync(token).ConfigureAwait(false);
        RaiseConfigChanged();
        return result;
    }

    /// <summary>Cached repository inspection; `force` re-downloads it.</summary>
    public async Task<RepoSnapshot?> SnapshotAsync(string url, bool force = false, CancellationToken token = default)
    {
        if (string.IsNullOrWhiteSpace(url))
        {
            return null;
        }
        if (!force && _snapshots.TryGetValue(url, out var cached))
        {
            return cached;
        }
        try
        {
            using var document = await _cli
                .RunJsonAsync(["inspect", url, "--images"], token)
                .ConfigureAwait(false);
            var snapshot = RepoSnapshot.FromJson(document);
            _snapshots[url] = snapshot;
            return snapshot;
        }
        catch (Exception)
        {
            return null;
        }
    }

    public void ForgetSnapshots() => _snapshots.Clear();

    public async Task<StatsSnapshot?> StatsAsync(string? workspace = null, CancellationToken token = default)
    {
        var args = new List<string> { "stats" };
        if (!string.IsNullOrWhiteSpace(workspace))
        {
            args.Add("--path");
            args.Add(workspace);
        }
        try
        {
            using var document = await _cli.RunJsonAsync(args, token).ConfigureAwait(false);
            return StatsSnapshot.FromJson(document);
        }
        catch (Exception)
        {
            return null;
        }
    }
}

