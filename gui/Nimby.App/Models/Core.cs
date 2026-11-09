using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;

namespace Nimby.App.Models;

public sealed class RepoRow
{
    public string Name { get; init; } = "";
    public string Url { get; init; } = "";
    public bool Active { get; init; }
    public bool HasPassword { get; init; }
    /// Subfolder of the workspace root this repository installs into.
    public string Folder { get; init; } = "";
    /// Full install folder: `<workspace root>/<folder>`.
    public string Path { get; init; } = "";
    /// True when the folder name is derived from the repository name.
    public bool AutoFolder { get; init; } = true;

    public string Badge => Active ? "active" : "";
    public string Detail => $"{Url}\n→ {Path}";
    public string PathText => $"{Folder}/  →  {Path}" + (AutoFolder ? "  (derived)" : "");
    public string ActiveText => Active ? "active" : "not active";
}

public sealed class ParamRow
{
    public string Key { get; init; } = "";
    public string? RawValue { get; set; }
    public string Label { get; init; } = "";
    public string Kind { get; init; } = "text";
    public string Display { get; init; } = "";
    public bool Known { get; init; }
    public bool Managed { get; init; }
    public bool Enabled { get; set; } = true;

    public string ValueText
    {
        get => RawValue ?? "";
        set => RawValue = string.IsNullOrWhiteSpace(value) ? null : value;
    }

    public string Subtitle => Known
        ? $"{Label} · {Display}"
        : Display;

    public bool NeedsValue => Kind is "text" or "number" or "path" or "choice";
}

/// <summary>
/// One mod in the install list. Besides the static manifest data it carries the
/// live install state, so the list can show a per-mod bar and status while the
/// run walks through it.
/// </summary>
public sealed class ModRow : System.ComponentModel.INotifyPropertyChanged
{
    public string Name { get; init; } = "";
    public string Checksum { get; init; } = "";
    public long Bytes { get; init; }
    public int Files { get; init; }
    public bool IsOptional { get; init; }
    public bool Selected { get; set; } = true;

    public string Size => FormatBytes(Bytes);

    public string Subtitle => Files > 0
        ? $"{Checksum[..Math.Min(8, Checksum.Length)]} · {Files} files · {Size}" +
          (IsOptional ? " · optional" : "")
        : $"{Checksum} · {Size}" + (IsOptional ? " · optional" : "");

    // ------------------------------------------------------------ live state

    private double _progress;
    private string _status = "";
    private string _progressText = "";
    private bool _active;
    private bool _done;
    private bool _failed;
    private bool _showProgress;

    public event System.ComponentModel.PropertyChangedEventHandler? PropertyChanged;

    public double Progress
    {
        get => _progress;
        set
        {
            if (Math.Abs(_progress - value) < 0.01)
            {
                return;
            }
            _progress = value;
            Raise(nameof(Progress));
        }
    }

    public string StatusText
    {
        get => _status;
        set => Set(ref _status, value, nameof(StatusText));
    }

    public string ProgressText
    {
        get => _progressText;
        set => Set(ref _progressText, value, nameof(ProgressText));
    }

    public bool IsActive
    {
        get => _active;
        set
        {
            if (_active == value)
            {
                return;
            }
            _active = value;
            Raise(nameof(IsActive));
            Raise(nameof(RowBackground));
        }
    }

    public bool IsDone
    {
        get => _done;
        set
        {
            if (_done == value)
            {
                return;
            }
            _done = value;
            Raise(nameof(IsDone));
            Raise(nameof(RowBackground));
        }
    }

    public bool IsFailed
    {
        get => _failed;
        set
        {
            if (_failed == value)
            {
                return;
            }
            _failed = value;
            Raise(nameof(IsFailed));
            Raise(nameof(StatusBrush));
        }
    }

    public bool ShowProgress
    {
        get => _showProgress;
        set
        {
            if (_showProgress == value)
            {
                return;
            }
            _showProgress = value;
            Raise(nameof(ShowProgress));
        }
    }

    /// <summary>Highlight the row that is being worked on right now.</summary>
    public Avalonia.Media.IBrush RowBackground => IsActive
        ? new Avalonia.Media.SolidColorBrush(Avalonia.Media.Color.Parse("#1B2430"))
        : IsDone
            ? new Avalonia.Media.SolidColorBrush(Avalonia.Media.Color.Parse("#141A14"))
            : Avalonia.Media.Brushes.Transparent;

    public Avalonia.Media.IBrush StatusBrush => IsFailed
        ? new Avalonia.Media.SolidColorBrush(Avalonia.Media.Color.Parse("#E5484D"))
        : IsActive
            ? new Avalonia.Media.SolidColorBrush(Avalonia.Media.Color.Parse("#F0B429"))
            : IsDone
                ? new Avalonia.Media.SolidColorBrush(Avalonia.Media.Color.Parse("#6DBE7B"))
                : Avalonia.Media.Brushes.Gray;

    /// <summary>Back to "queued" before an install run.</summary>
    public void ResetProgress()
    {
        Progress = 0;
        ProgressText = "";
        StatusText = "";
        IsActive = false;
        IsDone = false;
        IsFailed = false;
        ShowProgress = Bytes > 0;
    }

    private void Set(ref string field, string value, string name)
    {
        if (field == value)
        {
            return;
        }
        field = value;
        Raise(name);
    }

    private void Raise(string name) =>
        PropertyChanged?.Invoke(this, new System.ComponentModel.PropertyChangedEventArgs(name));

    public static string FormatBytes(long bytes)
    {
        string[] units = ["B", "KB", "MB", "GB", "TB"];
        double value = bytes;
        var unit = 0;
        while (value >= 1024 && unit < units.Length - 1)
        {
            value /= 1024;
            unit++;
        }
        return unit == 0 ? $"{bytes} B" : $"{value:0.0} {units[unit]}";
    }
}

public sealed class KnownParam
{
    public string Key { get; init; } = "";
    public string Label { get; init; } = "";
    public string Kind { get; init; } = "text";
    public string Description { get; init; } = "";
    public List<string> Choices { get; init; } = [];

    public string Display => $"{Key}  —  {Label}";

    public override string ToString() => Display;
}

public sealed class RepoStatsRow
{
    public string Repo { get; init; } = "";
    public int Mods { get; init; }
    public long Logical { get; init; }
    /// Store payload plus workspace copies. Workshop bytes belong to Steam.
    public long OnDisk { get; init; }
    /// Bytes the workspace folder itself holds.
    public long InWorkspace { get; init; }
    /// Bytes fetched from the repository (unique to this machine).
    public long Downloaded { get; init; }
    public long Saved { get; init; }

    public string LogicalText => ModRow.FormatBytes(Logical);
    public string OnDiskText => ModRow.FormatBytes(OnDisk);
    public string InWorkspaceText => ModRow.FormatBytes(InWorkspace);
    public string DownloadedText => ModRow.FormatBytes(Downloaded);
    public string SavedText => ModRow.FormatBytes(Saved);
    public string ModsText => Mods.ToString();
}

public sealed class RepoSnapshot
{
    public string Name { get; init; } = "";
    public string Version { get; init; } = "";
    public string ClientParameters { get; init; } = "";
    public string? BannerPath { get; init; }
    public string? IconPath { get; init; }
    public List<ModRow> Mods { get; init; } = [];
    public List<string> Servers { get; init; } = [];

    public long TotalBytes => Mods.Sum(m => m.Bytes);

    public string Summary =>
        $"{Name}{(string.IsNullOrEmpty(Version) ? "" : $" {Version}")} · {Mods.Count} mods · " +
        ModRow.FormatBytes(TotalBytes);

    public static RepoSnapshot FromJson(JsonDocument document)
    {
        var root = document.RootElement;
        var snapshot = new RepoSnapshot
        {
            Name = Json.Str(root, "repoName"),
            Version = Json.Str(root, "version"),
            ClientParameters = Json.Str(root, "clientParameters"),
        };

        var mods = new List<ModRow>();
        if (root.TryGetProperty("mods", out var modArray) && modArray.ValueKind == JsonValueKind.Array)
        {
            foreach (var mod in modArray.EnumerateArray())
            {
                mods.Add(new ModRow
                {
                    Name = Json.Str(mod, "mod"),
                    Checksum = Json.Str(mod, "checksum"),
                    Files = (int)Json.Num(mod, "files"),
                    Bytes = Json.Num(mod, "bytes"),
                    IsOptional = mod.TryGetProperty("optional", out var optional)
                                 && optional.ValueKind == JsonValueKind.True,
                    Selected = !mod.TryGetProperty("enabled", out var enabled)
                               || enabled.ValueKind != JsonValueKind.False,
                });
            }
        }

        var servers = new List<string>();
        if (root.TryGetProperty("servers", out var serverArray) && serverArray.ValueKind == JsonValueKind.Array)
        {
            foreach (var server in serverArray.EnumerateArray())
            {
                var name = Json.Str(server, "name");
                if (!string.IsNullOrEmpty(name))
                {
                    servers.Add(name);
                }
            }
        }

        string? banner = null;
        string? icon = null;
        if (root.TryGetProperty("images", out var images) && images.ValueKind == JsonValueKind.Object)
        {
            banner = Json.StrOrNull(images, "repo");
            icon = Json.StrOrNull(images, "icon");
        }

        return new RepoSnapshot
        {
            Name = snapshot.Name,
            Version = snapshot.Version,
            ClientParameters = snapshot.ClientParameters,
            Mods = mods,
            Servers = servers,
            BannerPath = banner,
            IconPath = icon,
        };
    }
}

public sealed class StatsSnapshot
{
    public string Workspace { get; init; } = "";
    public bool StoreEnabled { get; init; }
    public string StorePath { get; init; } = "";
    public long StoreBytes { get; init; }
    public int StoreMods { get; init; }
    public List<RepoStatsRow> Repos { get; init; } = [];
    public long TotalLogical { get; init; }
    public long TotalOnDisk { get; init; }
    public long TotalInWorkspace { get; init; }
    public long TotalSaved { get; init; }
    /// Bytes fetched from the repository (the tracking index knows).
    public long TotalDownloaded { get; init; }
    public int TrackedMods { get; init; }
    public int TrackedFiles { get; init; }

    public string WorkspaceText => Workspace;
    public string StoreText => StoreEnabled
        ? $"{StorePath} · {StoreMods} mods · {ModRow.FormatBytes(StoreBytes)}"
        : "disabled";
    public string LogicalText => ModRow.FormatBytes(TotalLogical);
    public string OnDiskText => ModRow.FormatBytes(TotalOnDisk);
    public string InWorkspaceText => ModRow.FormatBytes(TotalInWorkspace);
    public string SavedText => ModRow.FormatBytes(TotalSaved);
    public string DownloadedText => ModRow.FormatBytes(TotalDownloaded);
    public string TrackingText => TrackedMods == 0
        ? "tracking: none yet (install or repair to backfill)"
        : $"tracking: {TrackedMods} mods · {TrackedFiles} files indexed";

    public static StatsSnapshot FromJson(JsonDocument document)
    {
        var root = document.RootElement;
        var snapshot = new StatsSnapshot
        {
            Workspace = Json.Str(root, "workspace"),
        };

        string storePath = "";
        long storeBytes = 0;
        int storeMods = 0;
        var storeEnabled = false;
        if (root.TryGetProperty("store", out var store) && store.ValueKind == JsonValueKind.Object)
        {
            storeEnabled = true;
            storePath = Json.Str(store, "path");
            storeBytes = Json.Num(store, "bytes");
            storeMods = (int)Json.Num(store, "mods");
        }

        var repos = new List<RepoStatsRow>();
        long logical = 0, onDisk = 0, inWorkspace = 0, saved = 0, downloaded = 0;
        if (root.TryGetProperty("repos", out var repoArray) && repoArray.ValueKind == JsonValueKind.Array)
        {
            foreach (var repo in repoArray.EnumerateArray())
            {
                var repoLogical = Json.Num(repo, "logicalBytes");
                var repoWorkshop = Json.Num(repo, "workshopBytes");
                var repoStore = Json.Num(repo, "storeBytes");
                var repoDownloaded = Json.Num(repo, "downloadedBytes");
                var repoSaved = repoWorkshop + repoStore;
                // The store payload is real disk usage, so it counts here; only
                // the Workshop bytes belong to Steam.
                var repoOnDisk = Math.Max(0, repoLogical - repoWorkshop);
                var repoInWorkspace = Math.Max(0, repoLogical - repoSaved);
                logical += repoLogical;
                onDisk += repoOnDisk;
                inWorkspace += repoInWorkspace;
                saved += repoSaved;
                downloaded += repoDownloaded;
                repos.Add(new RepoStatsRow
                {
                    Repo = Json.Str(repo, "repo"),
                    Mods = (int)Json.Num(repo, "mods"),
                    Logical = repoLogical,
                    OnDisk = repoOnDisk,
                    InWorkspace = repoInWorkspace,
                    Downloaded = repoDownloaded,
                    Saved = repoSaved,
                });
            }
        }

        var trackedMods = 0;
        var trackedFiles = 0;
        if (root.TryGetProperty("tracking", out var tracking) && tracking.ValueKind == JsonValueKind.Object)
        {
            trackedMods = (int)Json.Num(tracking, "mods");
            trackedFiles = (int)Json.Num(tracking, "files");
        }

        return new StatsSnapshot
        {
            Workspace = snapshot.Workspace,
            StoreEnabled = storeEnabled,
            StorePath = storePath,
            StoreBytes = storeBytes,
            StoreMods = storeMods,
            Repos = repos,
            TotalLogical = logical,
            TotalOnDisk = onDisk,
            TotalInWorkspace = inWorkspace,
            TotalSaved = saved,
            TotalDownloaded = downloaded,
            TrackedMods = trackedMods,
            TrackedFiles = trackedFiles,
        };
    }
}

public sealed class ConfigSnapshot
{
    public string ConfigPath { get; init; } = "";
    public string? ArmaPath { get; init; }
    public string? DetectedArma { get; init; }
    public string? EffectiveArma { get; init; }
    public bool ArmaValid { get; init; }
    public string? TempPath { get; init; }
    public string EffectiveTemp { get; init; } = "";
    public string? SharedPath { get; init; }
    public long StoreBytes { get; init; }
    public int StoreMods { get; init; }
    /// Root folder: every repository gets its own subfolder in here.
    public string WorkspacePath { get; init; } = "";
    /// Install folder of the selected repository.
    public string ActiveWorkspace { get; init; } = "";
    public string LinkMode { get; init; } = "auto";
    public int ParallelDownloads { get; init; } = 8;
    /// How many mods are installed at the same time.
    public int ParallelMods { get; init; } = 2;
    public bool Battleye { get; init; }
    public string? ActiveRepo { get; init; }
    public List<ParamRow> Params { get; init; } = [];
    public List<RepoRow> Repos { get; init; } = [];
    public List<string> LinkModes { get; init; } = [];
    public List<KnownParam> KnownParams { get; init; } = [];

    public string ArmaText => EffectiveArma ?? "(not found)";
    public string ArmaSource => ArmaPath is null ? "autodetected" : "configured";
    public bool SharedEnabled => !string.IsNullOrEmpty(SharedPath);

    public static ConfigSnapshot FromJson(JsonDocument document)
    {
        var root = document.RootElement;

        var paramsList = new List<ParamRow>();
        if (root.TryGetProperty("params", out var paramsArray) && paramsArray.ValueKind == JsonValueKind.Array)
        {
            foreach (var param in paramsArray.EnumerateArray())
            {
                paramsList.Add(new ParamRow
                {
                    Key = Json.Str(param, "key"),
                    RawValue = Json.StrOrNull(param, "value"),
                    Label = Json.Str(param, "label"),
                    Kind = Json.Str(param, "kind"),
                    Display = Json.Str(param, "display"),
                    Known = param.TryGetProperty("known", out var known) && known.ValueKind == JsonValueKind.True,
                    Managed = Json.Str(param, "management") == "auto",
                    Enabled = !param.TryGetProperty("enabled", out var enabled) || enabled.ValueKind != JsonValueKind.False,
                });
            }
        }

        var repos = new List<RepoRow>();
        if (root.TryGetProperty("repos", out var repoArray) && repoArray.ValueKind == JsonValueKind.Array)
        {
            foreach (var repo in repoArray.EnumerateArray())
            {
                repos.Add(new RepoRow
                {
                    Name = Json.Str(repo, "name"),
                    Url = Json.Str(repo, "url"),
                    Active = repo.TryGetProperty("active", out var active) && active.ValueKind == JsonValueKind.True,
                    HasPassword = repo.TryGetProperty("hasPassword", out var pw) && pw.ValueKind == JsonValueKind.True,
                    Folder = Json.Str(repo, "folder"),
                    Path = Json.Str(repo, "path"),
                    AutoFolder = !repo.TryGetProperty("autoFolder", out var isAuto)
                                 || isAuto.ValueKind == JsonValueKind.True,
                });
            }
        }

        var knownParams = new List<KnownParam>();
        if (root.TryGetProperty("knownParams", out var knownArray) && knownArray.ValueKind == JsonValueKind.Array)
        {
            foreach (var spec in knownArray.EnumerateArray())
            {
                var choices = new List<string>();
                if (spec.TryGetProperty("choices", out var choiceArray) && choiceArray.ValueKind == JsonValueKind.Array)
                {
                    foreach (var choice in choiceArray.EnumerateArray())
                    {
                        if (choice.GetString() is { } text)
                        {
                            choices.Add(text);
                        }
                    }
                }
                knownParams.Add(new KnownParam
                {
                    Key = Json.Str(spec, "key"),
                    Label = Json.Str(spec, "label"),
                    Kind = Json.Str(spec, "kind"),
                    Description = Json.Str(spec, "description"),
                    Choices = choices,
                });
            }
        }

        var linkModes = new List<string>();
        if (root.TryGetProperty("linkModes", out var modeArray) && modeArray.ValueKind == JsonValueKind.Array)
        {
            foreach (var mode in modeArray.EnumerateArray())
            {
                if (mode.GetString() is { } text)
                {
                    linkModes.Add(text);
                }
            }
        }

        var storeBytes = 0L;
        var storeMods = 0;
        if (root.TryGetProperty("sharedStats", out var storeStats) && storeStats.ValueKind == JsonValueKind.Object)
        {
            storeBytes = Json.Num(storeStats, "bytes");
            storeMods = (int)Json.Num(storeStats, "mods");
        }

        return new ConfigSnapshot
        {
            ConfigPath = Json.Str(root, "configPath"),
            ArmaPath = Json.StrOrNull(root, "armaPath"),
            DetectedArma = Json.StrOrNull(root, "detectedArma"),
            EffectiveArma = Json.StrOrNull(root, "effectiveArma"),
            ArmaValid = root.TryGetProperty("armaValid", out var valid) && valid.ValueKind == JsonValueKind.True,
            TempPath = Json.StrOrNull(root, "tempPath"),
            EffectiveTemp = Json.Str(root, "effectiveTemp"),
            SharedPath = Json.StrOrNull(root, "sharedPath"),
            StoreBytes = storeBytes,
            StoreMods = storeMods,
            WorkspacePath = Json.Str(root, "workspacePath"),
            ActiveWorkspace = Json.Str(root, "activeWorkspace"),
            LinkMode = Json.Str(root, "linkMode"),
            ParallelDownloads = (int)Json.Num(root, "parallelDownloads"),
            ParallelMods = (int)Json.Num(root, "parallelMods"),
            Battleye = root.TryGetProperty("battleye", out var be) && be.ValueKind == JsonValueKind.True,
            ActiveRepo = Json.StrOrNull(root, "activeRepo"),
            Params = paramsList,
            Repos = repos,
            LinkModes = linkModes,
            KnownParams = knownParams,
        };
    }
}

internal static class Json
{
    public static string Str(JsonElement element, string name) =>
        element.TryGetProperty(name, out var value) && value.ValueKind == JsonValueKind.String
            ? value.GetString() ?? ""
            : "";

    public static string? StrOrNull(JsonElement element, string name) =>
        element.TryGetProperty(name, out var value) && value.ValueKind == JsonValueKind.String
            ? value.GetString()
            : null;

    public static long Num(JsonElement element, string name) =>
        element.TryGetProperty(name, out var value) && value.ValueKind == JsonValueKind.Number
            ? value.GetInt64()
            : 0;
}
