using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;

namespace Nimby.App.Services;

/// <summary>Exit codes `nimby install` uses so the UI can react precisely.</summary>
public static class CliExitCodes
{
    public const int Ok = 0;
    /// <summary>User cancelled; everything installed so far is kept.</summary>
    public const int Cancelled = 3;
    /// <summary>Finished, but at least one mod was skipped.</summary>
    public const int Incomplete = 4;
}

/// <summary>
/// Thin wrapper around the `nimby` executable. The GUI never re-implements
/// install logic: it drives the same CLI the tests cover, using --json output.
/// </summary>
public sealed class CliRunner
{
    private readonly string _executable;

    public CliRunner(string executable) => _executable = executable;

    public string Executable => _executable;

    /// <summary>Find nimby.exe next to the app, in this repo's build output, or on PATH.</summary>
    public static CliRunner Locate()
    {
        var candidates = new List<string>();
        var env = Environment.GetEnvironmentVariable("NIMBY_CLI");
        if (!string.IsNullOrWhiteSpace(env))
        {
            candidates.Add(env);
        }

        var appDir = AppContext.BaseDirectory;
        candidates.Add(Path.Combine(appDir, "nimby.exe"));
        candidates.Add(Path.Combine(appDir, "nimby"));

        var dir = new DirectoryInfo(appDir);
        for (var i = 0; i < 8 && dir is not null; i++, dir = dir.Parent)
        {
            candidates.Add(Path.Combine(dir.FullName, "target", "release", "nimby.exe"));
            candidates.Add(Path.Combine(dir.FullName, "target", "debug", "nimby.exe"));
        }

        var path = Environment.GetEnvironmentVariable("PATH") ?? string.Empty;
        foreach (var folder in path.Split(Path.PathSeparator, StringSplitOptions.RemoveEmptyEntries))
        {
            candidates.Add(Path.Combine(folder, "nimby.exe"));
            candidates.Add(Path.Combine(folder, "nimby"));
        }

        foreach (var candidate in candidates)
        {
            if (File.Exists(candidate))
            {
                return new CliRunner(candidate);
            }
        }

        throw new FileNotFoundException(
            "nimby.exe not found. Build it with `cargo build --release` or set NIMBY_CLI.");
    }

    public sealed record RunResult(int ExitCode, string StdOut, string StdErr)
    {
        public bool Ok => ExitCode == 0;
    }

    public Task<RunResult> RunAsync(IEnumerable<string> args, CancellationToken token = default) =>
        RunAsync(args, null, token);

    public async Task<RunResult> RunAsync(
        IEnumerable<string> args,
        Action<string>? onLine,
        CancellationToken token = default)
    {
        using var session = StartSession(args, onLine);
        var code = await session.Completion.WaitAsync(token).ConfigureAwait(false);
        return new RunResult(code, session.StdOut, session.StdErr);
    }

    /// Start the CLI with a writable stdin, so the caller can send control words
    /// (`cancel`, `retry`, `skip`, `abort`) while it runs.
    public CliSession StartSession(IEnumerable<string> args, Action<string>? onLine)
    {
        var start = new ProcessStartInfo(_executable)
        {
            RedirectStandardInput = true,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            UseShellExecute = false,
            CreateNoWindow = true,
        };
        foreach (var arg in args)
        {
            start.ArgumentList.Add(arg);
        }
        return new CliSession(start, onLine);
    }

    public async Task<JsonDocument> RunJsonAsync(IEnumerable<string> args, CancellationToken token = default)
    {
        var all = new List<string>(args) { "--json" };
        var result = await RunAsync(all, token).ConfigureAwait(false);
        if (!result.Ok)
        {
            throw new InvalidOperationException(
                string.IsNullOrWhiteSpace(result.StdErr) ? "nimby failed" : result.StdErr.Trim());
        }

        var text = result.StdOut.Trim();
        var start = text.IndexOf('{');
        if (start > 0)
        {
            text = text[start..];
        }
        return JsonDocument.Parse(text);
    }
}

/// <summary>
/// A running CLI process whose stdin stays open. The install page uses it to
/// send `cancel` and to answer `needsAction` events, so the GUI and the CLI
/// share one state machine instead of guessing about each other.
/// </summary>
public sealed class CliSession : IDisposable
{
    private readonly Process _process;
    private readonly System.Text.StringBuilder _stdout = new();
    private readonly System.Text.StringBuilder _stderr = new();
    private readonly object _stdinLock = new();
    private bool _disposed;
    private Task<int>? _completion;

    public CliSession(ProcessStartInfo start, Action<string>? onLine)
    {
        _process = new Process { StartInfo = start, EnableRaisingEvents = true };
        _process.OutputDataReceived += (_, e) =>
        {
            if (e.Data is null)
            {
                return;
            }
            lock (_stdout)
            {
                _stdout.AppendLine(e.Data);
            }
            onLine?.Invoke(e.Data);
        };
        _process.ErrorDataReceived += (_, e) =>
        {
            if (e.Data is null)
            {
                return;
            }
            lock (_stderr)
            {
                _stderr.AppendLine(e.Data);
            }
            onLine?.Invoke(e.Data);
        };
        _process.Start();
        _process.StandardInput.AutoFlush = true;
        _process.BeginOutputReadLine();
        _process.BeginErrorReadLine();
    }

    public int ProcessId => _process.Id;

    public string StdOut
    {
        get { lock (_stdout) { return _stdout.ToString(); } }
    }

    public string StdErr
    {
        get { lock (_stderr) { return _stderr.ToString(); } }
    }

    public Task<int> Completion => _completion ??= WaitForExitAsync();

    private async Task<int> WaitForExitAsync()
    {
        await _process.WaitForExitAsync().ConfigureAwait(false);
        return _process.ExitCode;
    }

    public int ExitCode => _process.ExitCode;

    /// <summary>Send a control word: cancel, retry, skip or abort.</summary>
    public bool Write(string line)
    {
        if (_disposed)
        {
            return false;
        }
        try
        {
            lock (_stdinLock)
            {
                _process.StandardInput.WriteLine(line);
            }
            return true;
        }
        catch (Exception)
        {
            return false;
        }
    }

    public void Dispose()
    {
        if (_disposed)
        {
            return;
        }
        _disposed = true;
        try
        {
            _process.StandardInput.Close();
        }
        catch (Exception)
        {
            // Process already gone.
        }
        try
        {
            if (!_process.HasExited)
            {
                _process.Kill(entireProcessTree: true);
            }
        }
        catch (Exception)
        {
            // Nothing left to stop.
        }
        _process.Dispose();
    }
}
