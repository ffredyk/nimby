using System;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Threading.Tasks;
using Avalonia.Controls;
using Avalonia.Interactivity;
using Avalonia.Threading;
using Nimby.App.Models;
using Nimby.App.Services;

namespace Nimby.App.Views;

public partial class InstallView : UserControl, IRefreshable
{
    private readonly AppState _state;
    private readonly ObservableCollection<ModRow> _mods = [];
    private readonly Dictionary<string, ModRow> _rowsByName = new(StringComparer.OrdinalIgnoreCase);
    /// Live per-mod state. Events arrive on the reader thread and are folded
    /// here under a lock, so a `modDone` can never be overtaken by an older
    /// coalesced `file` event; the flush timer paints the rows from this map.
    private readonly Dictionary<string, ModLive> _live = new(StringComparer.OrdinalIgnoreCase);
    private readonly object _liveLock = new();
    private string? _currentMod;
    private string? _scrolledTo;
    private RepoSnapshot? _snapshot;

    public InstallView(AppState state)
    {
        _state = state;
        InitializeComponent();
        ModList.ItemsSource = _mods;
        _flushTimer = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(80) };
        _flushTimer.Tick += (_, _) => FlushPending();
    }

    private readonly DispatcherTimer _flushTimer;

    public async Task RefreshAsync() => await LoadAsync(false);

    /// <summary>Last status line, for the headless smoke test.</summary>
    public string CurrentStatus => StatusLine.Text ?? "";

    /// <summary>Progress panel contents, for the headless smoke test.</summary>
    public string ProgressSnapshot() =>
        $"{ProgressTitle.Text} | {ProgressPercent.Text} | {ProgressDetail.Text} | {ProgressBytes.Text}";

    /// <summary>One mod row, for the headless smoke test.</summary>
    public string RowSnapshot(string modName)
    {
        var row = RowFor(modName);
        return row is null
            ? $"{modName}: (missing row)"
            : $"{row.Name} | {row.Progress:0}% | {row.ProgressText} | {row.StatusText} | " +
              $"active={row.IsActive} done={row.IsDone}";
    }

    /// <summary>
    /// CLI output arrives on the thread pool, so every UI touch is marshalled.
    /// Append is batched: a mod with thousands of files must not rebuild the
    /// log TextBlock thousands of times.
    /// </summary>
    private void Log(string line)
    {
        if (Dispatcher.UIThread.CheckAccess())
        {
            AppendLog(line);
            return;
        }
        lock (_pendingLock)
        {
            if (_pendingLog.Count < 2000)
            {
                _pendingLog.Add(line);
            }
        }
    }

    private void AppendLog(string line)
    {
        _logLines.Add(line);
        if (_logLines.Count > MaxLogLines)
        {
            _logLines.RemoveRange(0, _logLines.Count - MaxLogLines);
        }
        LogBox.Text = string.Join(Environment.NewLine, _logLines);
    }

    private void FlushLog()
    {
        List<string>? pending = null;
        lock (_pendingLock)
        {
            if (_pendingLog.Count > 0)
            {
                pending = [.. _pendingLog];
                _pendingLog.Clear();
            }
        }
        if (pending is null || pending.Count == 0)
        {
            return;
        }
        _logLines.AddRange(pending);
        if (_logLines.Count > MaxLogLines)
        {
            _logLines.RemoveRange(0, _logLines.Count - MaxLogLines);
        }
        LogBox.Text = string.Join(Environment.NewLine, _logLines);
    }

    private const int MaxLogLines = 300;

    private async Task LoadAsync(bool force)
    {
        await _state.RefreshConfigAsync();
        var url = _state.ActiveRepoUrl;
        if (string.IsNullOrEmpty(url))
        {
            RepoName.Text = "No repository selected";
            RepoSummary.Text = "Add a repository on the Repos tab.";
            RepoPathText.Text = "";
            _mods.Clear();
            IconImage.Source = null;
            BannerHost.IsVisible = false;
            return;
        }

        StatusLine.Text = "Loading repository…";
        _snapshot = await _state.SnapshotAsync(url, force);
        if (_snapshot is null)
        {
            StatusLine.Text = "Cannot read the repository. Check the URL on the Repos tab.";
            RepoPathText.Text = "";
            return;
        }

        RepoName.Text = _snapshot.Name;
        RepoSummary.Text = _snapshot.Summary;
        // The folder belongs to the repo, not to a global setting, so it is
        // shown right next to the repository it belongs to.
        RepoPathText.Text = $"installs into {_state.WorkspacePath}";
        IconImage.Source = Images.TryLoad(_snapshot.IconPath);
        var banner = Images.TryLoad(_snapshot.BannerPath);
        BannerImage.Source = banner;
        BannerHost.IsVisible = banner is not null;

        _mods.Clear();
        _rowsByName.Clear();
        foreach (var mod in _snapshot.Mods)
        {
            _mods.Add(mod);
            _rowsByName[mod.Name] = mod;
        }
        UpdateHeader();
        // A refresh rebuilds the row objects; repaint them from the folded event
        // state so a finished run keeps showing its result instead of resetting.
        ApplyRows();
        StatusLine.Text = _mods.Count == 0
            ? "Repository has no mods."
            : $"{_mods.Count} mods · {ModRow.FormatBytes(_snapshot.TotalBytes)}";
    }

    private void UpdateHeader() =>
        ModsHeader.Text = $"Mods ({_mods.Count(m => m.Selected)} of {_mods.Count} selected)";

    // ------------------------------------------------------- per-mod progress

    /// <summary>State of one mod while an install runs.</summary>
    private sealed class ModLive
    {
        public long DoneBytes;
        public int DoneFiles;
        public string Status = "";
        public string? File;
        public long FileBytes;
        public bool Active;
        public bool Done;
        public bool Failed;
    }

    private ModRow? RowFor(string modName) =>
        _rowsByName.TryGetValue(modName, out var row) ? row : null;

    /// <summary>Clear the folded state (any thread).</summary>
    private void ResetLiveState()
    {
        lock (_liveLock)
        {
            _live.Clear();
            _currentMod = null;
            _scrolledTo = null;
        }
    }

    /// <summary>Reset every row before a run so the list starts from scratch.</summary>
    private void ResetRows()
    {
        ResetLiveState();
        foreach (var row in _mods)
        {
            row.ResetProgress();
            row.StatusText = row.Selected ? "queued" : "";
            row.ShowProgress = row.Selected && row.Bytes > 0;
        }
    }

    private ModLive State(string mod)
    {
        if (!_live.TryGetValue(mod, out var state))
        {
            state = new ModLive();
            _live[mod] = state;
        }
        return state;
    }

    /// <summary>Fold one CLI event into the per-mod state (any thread).</summary>
    private void RecordRowEvent(string eventName, JsonElement root)
    {
        // `plan` carries no mod name: it starts a fresh run, so the folded state
        // must be cleared here (before any file event is folded in).
        if (eventName == "plan")
        {
            ResetLiveState();
            return;
        }
        if (eventName == "cancelled")
        {
            lock (_liveLock)
            {
                if (_currentMod is not null && _live.TryGetValue(_currentMod, out var stopped))
                {
                    stopped.Active = false;
                    stopped.File = null;
                    stopped.FileBytes = 0;
                    stopped.Status = "cancelled";
                }
            }
            return;
        }
        var mod = Str(root, "mod");
        if (string.IsNullOrEmpty(mod))
        {
            return;
        }
        lock (_liveLock)
        {
            var state = State(mod);
            switch (eventName)
            {
                case "modStart":
                    state.Active = true;
                    state.Done = false;
                    state.Failed = false;
                    state.Status = "starting";
                    _currentMod = mod;
                    break;
                case "fileStart":
                    state.File = Str(root, "path");
                    state.FileBytes = 0;
                    state.Status = $"fetching {Str(root, "path")}";
                    break;
                case "fileProgress":
                {
                    state.File = Str(root, "path");
                    state.FileBytes = Num(root, "fileBytes");
                    var rate = Speed(Num(root, "speed"));
                    state.Status = rate.Length > 0
                        ? $"{Str(root, "path")} · {rate}"
                        : $"{Str(root, "path")} — receiving";
                    break;
                }
                case "file":
                {
                    state.DoneBytes += Num(root, "bytes");
                    state.DoneFiles += 1;
                    state.File = null;
                    state.FileBytes = 0;
                    var fileSpeed = Speed(Num(root, "speed"));
                    state.Status = $"{ActionLabel(Str(root, "action"))} {Str(root, "path")}";
                    if (fileSpeed.Length > 0 && Str(root, "action") == "download")
                    {
                        state.Status += $" · {fileSpeed}";
                    }
                    break;
                }
                case "modDone":
                    state.Active = false;
                    state.Done = true;
                    state.File = null;
                    state.FileBytes = 0;
                    state.Status = $"{Str(root, "mode")} · saved {Size(Num(root, "savedBytes"))}";
                    break;
                case "resumed":
                    state.Status = $"kept {Num(root, "files")} file(s) from the previous run";
                    break;
                case "retry":
                    state.Status = $"retrying {Num(root, "attempt")}/{Num(root, "attempts")}";
                    break;
                case "needsAction":
                    state.Status = "needs your decision";
                    break;
                case "skipped":
                    state.Active = false;
                    state.Failed = true;
                    state.Status = "skipped";
                    break;
                case "failed":
                    state.Active = false;
                    state.Failed = true;
                    state.Status = $"failed: {Str(root, "message")}";
                    break;
            }
        }
    }

    /// <summary>Paint every row from the folded state (UI thread).</summary>
    private void ApplyRows()
    {
        List<(string Mod, long Bytes, int Files, string Status, bool Active, bool Done, bool Failed)>
            snapshot;
        string? current;
        lock (_liveLock)
        {
            snapshot = _live
                .Select(entry => (
                    Mod: entry.Key,
                    Bytes: entry.Value.DoneBytes + entry.Value.FileBytes,
                    Files: entry.Value.DoneFiles,
                    Status: entry.Value.Status,
                    Active: entry.Value.Active,
                    Done: entry.Value.Done,
                    Failed: entry.Value.Failed))
                .ToList();
            current = _currentMod;
        }

        foreach (var (mod, done, files, status, active, doneFlag, failed) in snapshot)
        {
            var row = RowFor(mod);
            if (row is null)
            {
                continue;
            }
            row.ShowProgress = row.Bytes > 0;
            if (row.IsActive != active)
            {
                row.IsActive = active;
            }
            if (row.IsDone != doneFlag)
            {
                row.IsDone = doneFlag;
            }
            if (row.IsFailed != failed)
            {
                row.IsFailed = failed;
            }
            row.StatusText = status;
            var percent = row.Bytes > 0
                ? Math.Clamp(done * 100.0 / row.Bytes, 0, 100)
                : (row.Files > 0 && files >= row.Files ? 100 : 0);
            if (doneFlag)
            {
                percent = 100;
            }
            row.Progress = percent;
            row.ProgressText = row.Files > 0
                ? $"{percent:0}% · {(doneFlag ? row.Files : files)}/{row.Files} files"
                : $"{percent:0}%";
        }

        // Follow the install: scroll when a different mod becomes active.
        if (current is not null && current != _scrolledTo)
        {
            _scrolledTo = current;
            var row = RowFor(current);
            if (row is not null)
            {
                ScrollToRow(row);
            }
        }
    }

    /// <summary>Bring a row into view without moving focus.</summary>
    private void ScrollToRow(ModRow row)
    {
        var index = _mods.IndexOf(row);
        if (index < 0)
        {
            return;
        }
        ModList.ContainerFromIndex(index)?.BringIntoView();
    }

    private static string Key(string mod, string path) => $"{mod}\u0001{path}";

    private async void OnReload(object? sender, RoutedEventArgs e)
    {
        Log("reloading repository…");
        await LoadAsync(true);
    }

    private void OnSelectAll(object? sender, RoutedEventArgs e)
    {
        foreach (var mod in _mods)
        {
            mod.Selected = true;
        }
        RefreshSelection();
    }

    private void OnSelectNone(object? sender, RoutedEventArgs e)
    {
        foreach (var mod in _mods)
        {
            mod.Selected = false;
        }
        RefreshSelection();
    }

    private void RefreshSelection()
    {
        // Rebind so checkboxes pick up the programmatic change.
        var copy = _mods.ToList();
        _mods.Clear();
        foreach (var mod in copy)
        {
            _mods.Add(mod);
        }
        UpdateHeader();
    }

    private async void OnInstall(object? sender, RoutedEventArgs e)
    {
        try
        {
            if (_snapshot is null)
            {
                await LoadAsync(false);
            }
            var selected = _mods.Where(m => m.Selected).Select(m => m.Name).ToList();
            await InstallModsAsync(selected);
        }
        catch (Exception ex)
        {
            StatusLine.Text = ex.Message;
            Log($"error: {ex.Message}");
        }
    }

    /// Install the given mods (empty means "everything selected").
    public async Task InstallModsAsync(IReadOnlyList<string> requested, bool noWorkshop = false)
    {
        var wanted = requested.Count > 0
            ? requested.ToHashSet(StringComparer.OrdinalIgnoreCase)
            : _mods.Where(m => m.Selected).Select(m => m.Name).ToHashSet(StringComparer.OrdinalIgnoreCase);

        var required = _mods
            .Where(m => !m.IsOptional && wanted.Contains(m.Name))
            .Select(m => m.Name)
            .ToList();
        var optional = _mods
            .Where(m => m.IsOptional && wanted.Contains(m.Name))
            .Select(m => m.Name)
            .ToList();
        if (required.Count == 0 && optional.Count == 0)
        {
            StatusLine.Text = "Select at least one mod.";
            return;
        }

        // Always be explicit: `--mods` lists the required mods to install and
        // `--no-required` means "none of them", which is not the same as
        // omitting the flag (that would install every required mod).
        var args = new List<string> { "install" };
        if (required.Count > 0)
        {
            args.Add("--mods");
            args.Add(string.Join(',', required));
        }
        else
        {
            args.Add("--no-required");
        }
        if (optional.Count > 0)
        {
            args.Add("--optional");
            args.Add(string.Join(',', optional));
        }
        args.Add("--progress");
        args.Add("json");
        // The CLI runs the error policy and asks us when it cannot continue.
        args.Add("--on-error");
        args.Add("ask");
        args.Add("--control-stdin");
        if (noWorkshop)
        {
            args.Add("--no-workshop");
        }

        BeginProgress();
        SetBusy(true, $"Installing {required.Count + optional.Count} mods…");
        _session = _state.Cli.StartSession(args, OnCliLine);
        try
        {
            await _session.Completion;
            var code = _session.ExitCode;
            StatusLine.Text = code switch
            {
                0 => "Install finished.",
                CliExitCodes.Cancelled =>
                    "Cancelled. Install again to resume — finished files are kept.",
                CliExitCodes.Incomplete => "Install finished with skipped mods. See the log.",
                _ => $"Install failed (exit {code}).",
            };
            if (code != 0)
            {
                ProgressTitle.Text = code == CliExitCodes.Cancelled ? "Cancelled" : "Install incomplete";
            }
        }
        catch (Exception ex)
        {
            StatusLine.Text = ex.Message;
            ProgressTitle.Text = "Install failed";
            ProgressDetail.Text = ex.Message;
            Log($"error: {ex.Message}");
        }
        finally
        {
            FlushPending();
            _flushTimer.Stop();
            HidePrompt();
            _session?.Dispose();
            _session = null;
            SetBusy(false, StatusLine.Text ?? "Ready.");
            ProgressBar.IsIndeterminate = false;
        }
    }

    // ---------------------------------------------------------------- control

    private CliSession? _session;
    private bool _awaitingDecision;

    private void OnCancel(object? sender, RoutedEventArgs e) => CancelInstall();

    /// <summary>
    /// Ask the CLI to stop after the file it is working on. Everything already
    /// verified stays on disk, so pressing Install again resumes.
    /// </summary>
    public void CancelInstall()
    {
        if (_session is null)
        {
            return;
        }
        _session.Write("cancel");
        StatusLine.Text = "Cancelling — finishing the current file…";
        CancelButton.IsEnabled = false;
        HidePrompt();
        Log("cancel requested");
    }

    private void OnPromptRetry(object? sender, RoutedEventArgs e) => AnswerPrompt("retry");

    private void OnPromptSkip(object? sender, RoutedEventArgs e) => AnswerPrompt("skip");

    private void OnPromptAbort(object? sender, RoutedEventArgs e) => AnswerPrompt("abort");

    private void AnswerPrompt(string answer)
    {
        if (_session is null || !_awaitingDecision)
        {
            return;
        }
        _awaitingDecision = false;
        HidePrompt();
        _session.Write(answer);
        StatusLine.Text = answer switch
        {
            "retry" => "Retrying…",
            "skip" => "Skipping this mod…",
            _ => "Stopping…",
        };
        Log($"decision: {answer}");
    }

    private void ShowPrompt(string mod, string message, string kind)
    {
        _awaitingDecision = true;
        PromptHost.IsVisible = true;
        PromptTitle.Text = $"{mod} needs a decision";
        PromptMessage.Text = kind switch
        {
            "network" => $"Cannot reach the repository: {message}",
            "checksum" => $"A downloaded file is corrupt: {message}",
            "disk" => $"The disk refused a write: {message}",
            "missing-source" => $"The repository does not offer this file: {message}",
            _ => message,
        };
        PromptRetryButton.IsVisible = true;
        PromptSkipButton.IsVisible = true;
        PromptAbortButton.IsVisible = true;
        ProgressNote.Text = "waiting for your choice";
        // Do not steal focus: the window may be in the background.
        TaskbarAttention.Request(GetWindow(), alsoFocus: false);
    }

    private void HidePrompt()
    {
        _awaitingDecision = false;
        PromptHost.IsVisible = false;
        PromptRetryButton.IsVisible = false;
        PromptSkipButton.IsVisible = false;
        PromptAbortButton.IsVisible = false;
        ProgressNote.Text = "";
        TaskbarAttention.Cancel(GetWindow());
    }

    private Window? GetWindow() => TopLevel.GetTopLevel(this) as Window;

    // ---------------------------------------------------------------- progress

    private void BeginProgress()
    {
        ProgressHost.IsVisible = true;
        ProgressBar.IsIndeterminate = true;
        ProgressBar.Value = 0;
        ProgressPercent.Text = "";
        ProgressTitle.Text = "Preparing…";
        ProgressDetail.Text = "Reading repository manifests";
        ProgressBytes.Text = "";
        HidePrompt();
        lock (_pendingLock)
        {
            _pendingEventName = null;
            _pendingEvent = default;
        }
        _flushTimer.Start();
    }

    private readonly object _pendingLock = new();
    private string? _pendingEventName;
    private JsonElement _pendingEvent;
    private readonly List<string> _pendingLog = [];
    private readonly List<string> _logLines = [];

    /// <summary>CLI stdout lines: progress events (JSON) or plain text.</summary>
    private void OnCliLine(string line)
    {
        var trimmed = line.Trim();
        if (trimmed.StartsWith('{'))
        {
            JsonDocument document;
            try
            {
                document = JsonDocument.Parse(trimmed);
            }
            catch (JsonException)
            {
                Log(line);
                return;
            }
            using (document)
            {
                var root = document.RootElement;
                var eventName = root.TryGetProperty("event", out var name) ? name.GetString() : null;
                if (eventName is not null)
                {
                    // Clone before the document is disposed: handlers run later.
                    var element = root.Clone();
                    // Per-mod bookkeeping happens here, on the reader thread, so
                    // the row state is complete even when painting is coalesced.
                    RecordRowEvent(eventName, element);
                    switch (eventName)
                    {
                        case "fileStart":
                        case "fileProgress":
                        case "file":
                        case "plan":
                        case "modStart":
                        case "resolving":
                            // High-frequency events: keep only the newest and let
                            // the flush timer paint the UI. Without this, a mod
                            // with thousands of files floods the dispatcher.
                            lock (_pendingLock)
                            {
                                _pendingEventName = eventName;
                                _pendingEvent = element;
                            }
                            break;
                            case "needsAction":
                                // Must not be coalesced: the CLI is blocked until the
                                // user answers.
                            Dispatcher.UIThread.Post(() => HandleEvent(eventName, element));
                            break;
                            case "modDone":
                            case "note":
                            case "resumed":
                            case "retry":
                            case "skipped":
                            case "failed":
                            case "cancelled":
                            case "done":
                                Dispatcher.UIThread.Post(() => HandleEvent(eventName, element));
                                break;
                            default:
                                Log(line);
                                break;
                        }
                    return;
                }
            }
        }
        Log(line);
    }

    private void FlushPending()
    {
        FlushLog();
        ApplyRows();
        string? name;
        JsonElement element;
        lock (_pendingLock)
        {
            name = _pendingEventName;
            element = _pendingEvent;
            _pendingEventName = null;
            _pendingEvent = default;
        }
        if (name is not null)
        {
            HandleEvent(name, element);
        }
    }

    private void HandleEvent(string eventName, JsonElement root)
    {
        switch (eventName)
        {
            case "resolving":
            {
                ProgressBar.IsIndeterminate = true;
                var mod = Str(root, "mod");
                var index = Num(root, "index");
                var count = Num(root, "count");
                ProgressTitle.Text = $"Reading manifests {index}/{count}";
                ProgressDetail.Text = mod;
                break;
            }
            case "plan":
            {
                ProgressBar.IsIndeterminate = false;
                ProgressBar.Value = 0;
                ProgressPercent.Text = "0%";
                ProgressTitle.Text = $"Installing {Num(root, "mods")} mods";
                ProgressDetail.Text = $"{Num(root, "files")} files";
                ProgressBytes.Text = $"{Size(0)} / {Size(Num(root, "totalBytes"))}";
                ResetRows();
                break;
            }
            case "modStart":
            {
                ProgressBar.IsIndeterminate = false;
                ProgressTitle.Text =
                    $"Installing {Num(root, "index")}/{Num(root, "count")} · {Str(root, "mod")}";
                ProgressDetail.Text = $"{Num(root, "files")} files · {Size(Num(root, "bytes"))}";
                break;
            }
            case "file":
            {
                var percent = Num(root, "percent");
                ProgressBar.IsIndeterminate = false;
                ProgressBar.Value = percent;
                ProgressPercent.Text = $"{percent}%";
                ProgressTitle.Text = Str(root, "mod");
                ProgressDetail.Text = $"{Str(root, "path")} — {ActionLabel(Str(root, "action"))}";
                ProgressBytes.Text =
                    $"{Size(Num(root, "doneBytes"))} / {Size(Num(root, "totalBytes"))} · " +
                    $"{Num(root, "doneFiles")}/{Num(root, "totalFiles")} files{ActiveSuffix(root)}" +
                    SpeedSuffix(root);
                break;
            }
            case "fileStart":
            {
                ProgressBar.IsIndeterminate = false;
                ProgressBar.Value = Num(root, "percent");
                ProgressPercent.Text = $"{Num(root, "percent")}%";
                ProgressTitle.Text = Str(root, "mod");
                ProgressDetail.Text = $"{Str(root, "path")} — fetching {Size(Num(root, "bytes"))}";
                ProgressBytes.Text =
                    $"{Size(Num(root, "doneBytes"))} / {Size(Num(root, "totalBytes"))}" +
                    ActiveSuffix(root) + SpeedSuffix(root);
                break;
            }
            case "fileProgress":
            {
                var percent = Num(root, "percent");
                ProgressBar.IsIndeterminate = false;
                ProgressBar.Value = percent;
                ProgressPercent.Text = $"{percent}%";
                ProgressTitle.Text = Str(root, "mod");
                ProgressDetail.Text = $"{Str(root, "path")} — receiving…";
                ProgressBytes.Text =
                    $"{Size(Num(root, "doneBytes"))} / {Size(Num(root, "totalBytes"))}" +
                    ActiveSuffix(root) + SpeedSuffix(root);
                break;
            }
            case "modDone":
                Log($"  {Str(root, "mod")} · {Str(root, "mode")} · saved {Size(Num(root, "savedBytes"))}");
                break;
            case "retry":
            {
                var delay = Num(root, "delayMs");
                var attempt = Num(root, "attempt");
                var attempts = Num(root, "attempts");
                var suffix = delay > 0 ? $" in {delay} ms" : "";
                Log($"retrying {Str(root, "mod")} (attempt {attempt}/{attempts}){suffix}: {Str(root, "message")}");
                ProgressNote.Text = $"retrying {attempt}/{attempts}{suffix}";
                break;
            }
            case "resumed":
                Log(
                    $"{Str(root, "mod")}: kept {Num(root, "files")} file(s) from the previous run " +
                    $"({Size(Num(root, "bytes"))})");
                break;
            case "needsAction":
                ShowPrompt(Str(root, "mod"), Str(root, "message"), Str(root, "kind"));
                Log($"needs a decision: {Str(root, "mod")}: {Str(root, "message")}");
                break;
            case "skipped":
                Log($"skipped {Str(root, "mod")}: {Str(root, "message")}");
                HidePrompt();
                break;
            case "failed":
                Log($"failed {Str(root, "mod")}: {Str(root, "message")}");
                ProgressTitle.Text = $"{Str(root, "mod")} failed";
                ProgressDetail.Text = Str(root, "message");
                HidePrompt();
                break;
            case "cancelled":
            {
                ProgressBar.IsIndeterminate = false;
                ProgressTitle.Text = "Cancelled";
                ProgressDetail.Text = "Install again to resume; finished files are kept";
                ProgressNote.Text = "";
                HidePrompt();
                break;
            }
            case "note":
                Log($"note: {Str(root, "message")}");
                break;
            case "done":
            {
                ProgressBar.IsIndeterminate = false;
                ProgressBar.Value = 100;
                ProgressPercent.Text = "100%";
                ProgressTitle.Text = $"Installed {Num(root, "mods")} mods";
                ProgressDetail.Text =
                    $"saved {Size(Num(root, "savedBytes"))} · downloaded {Size(Num(root, "downloadedBytes"))}";
                ProgressBytes.Text = $"{Size(Num(root, "doneBytes"))} / {Size(Num(root, "totalBytes"))}";
                break;
            }
            default:
                Log($"{eventName}: {root.GetRawText()}");
                break;
        }
    }

    private static string ActionLabel(string action) => action switch
    {
        "workshop" => "linked from Workshop",
        "store" => "linked from shared store",
        "download" => "downloaded",
        "copy" => "copied",
        _ => action,
    };

    /// <summary>` · 12.4 MB/s · ~1m 5s left` from a progress event.</summary>
    private static string SpeedSuffix(JsonElement root)
    {
        var speed = Num(root, "speed");
        var eta = Num(root, "etaSeconds");
        var text = "";
        if (speed > 0)
        {
            text += $" · {Speed(speed)}";
        }
        var etaText = Eta(eta);
        if (etaText.Length > 0 && speed > 0)
        {
            text += $" · {etaText}";
        }
        return text;
    }

    private static string ActiveSuffix(JsonElement root)
    {
        var active = Num(root, "active");
        return active > 1 ? $" · {active} in parallel" : "";
    }

    private static string Str(JsonElement element, string name) =>
        element.TryGetProperty(name, out var value) && value.ValueKind == JsonValueKind.String
            ? value.GetString() ?? ""
            : "";

    private static long Num(JsonElement element, string name) =>
        element.TryGetProperty(name, out var value) && value.ValueKind == JsonValueKind.Number
            ? value.GetInt64()
            : 0;

    private static string Size(long bytes) => ModRow.FormatBytes(bytes);

    /// <summary>Transfer rate, e.g. `12.4 MB/s`.</summary>
    private static string Speed(long bytesPerSecond) =>
        bytesPerSecond > 0 ? $"{ModRow.FormatBytes(bytesPerSecond)}/s" : "";

    /// <summary>Rough time left, e.g. `~1m 20s left`.</summary>
    private static string Eta(long seconds) => seconds switch
    {
        <= 0 => "",
        < 60 => $"~{seconds}s left",
        < 3600 => $"~{seconds / 60}m {seconds % 60}s left",
        _ => $"~{seconds / 3600}h {(seconds % 3600) / 60}m left",
    };

    private async void OnVerify(object? sender, RoutedEventArgs e)
    {
        var selected = _mods.Where(m => m.Selected).Select(m => m.Name).ToList();
        if (selected.Count == 0)
        {
            StatusLine.Text = "Select at least one mod.";
            return;
        }
        SetBusy(true, "Verifying…");
        try
        {
            var workspace = _state.WorkspacePath;
            var failures = 0;
            foreach (var mod in selected)
            {
                var path = Path.Combine(workspace, mod);
                var result = await _state.Cli.RunAsync(["verify", path], Log);
                if (!result.Ok)
                {
                    failures++;
                }
            }
            StatusLine.Text = failures == 0
                ? "All selected mods verified."
                : $"{failures} mod(s) need repair.";
        }
        catch (Exception ex)
        {
            StatusLine.Text = ex.Message;
            Log($"error: {ex.Message}");
        }
        finally
        {
            SetBusy(false, StatusLine.Text ?? "Ready.");
        }
    }

    private async void OnStatus(object? sender, RoutedEventArgs e)
    {
        SetBusy(true, "Reading workspace…");
        try
        {
            var result = await _state.Cli.RunAsync(["status"], Log);
            StatusLine.Text = result.Ok ? "Workspace listed below." : "Status failed.";
        }
        catch (Exception ex)
        {
            StatusLine.Text = ex.Message;
            Log($"error: {ex.Message}");
        }
        finally
        {
            SetBusy(false, StatusLine.Text ?? "Ready.");
        }
    }

    private void SetBusy(bool busy, string status)
    {
        InstallButton.IsEnabled = !busy;
        ReloadButton.IsEnabled = !busy;
        CancelButton.IsVisible = busy;
        CancelButton.IsEnabled = busy;
        StatusLine.Text = status;
    }
}
