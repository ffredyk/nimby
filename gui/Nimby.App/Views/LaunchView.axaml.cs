using System;
using System.Collections.Generic;
using System.Threading.Tasks;
using Avalonia.Controls;
using Avalonia.Interactivity;
using Nimby.App.Models;
using Nimby.App.Services;

namespace Nimby.App.Views;

public partial class LaunchView : UserControl, IRefreshable
{
    private readonly AppState _state;

    public LaunchView(AppState state)
    {
        _state = state;
        InitializeComponent();
    }

    public async Task RefreshAsync()
    {
        await _state.RefreshConfigAsync();
        var config = _state.Config;
        if (config is null)
        {
            StatusLine.Text = "Cannot read the configuration.";
            return;
        }

        ArmaText.Text = config.ArmaValid
            ? $"Arma 3: {config.ArmaText} ({config.ArmaSource})"
            : "Arma 3 not found. Set the path on the Configuration tab.";

        BattleyeBox.IsChecked = config.Battleye;
        ParamsText.Text = config.Params.Count == 0
            ? "No launch parameters configured."
            : $"{config.Params.Count} launch parameter(s) configured on the Parameters tab.";
        StoreText.Text = config.SharedEnabled
            ? $"Shared store: {config.SharedPath}"
            : "Shared store disabled.";

        var snapshot = await _state.SnapshotAsync(_state.ActiveRepoUrl);
        var servers = new List<string>();
        if (snapshot is not null)
        {
            servers.AddRange(snapshot.Servers);
        }
        var previous = ServerBox.SelectedItem as string;
        ServerBox.ItemsSource = servers;
        if (previous is not null && servers.Contains(previous))
        {
            ServerBox.SelectedItem = previous;
        }
        else if (servers.Count > 0)
        {
            ServerBox.SelectedIndex = 0;
        }

        await PreviewAsync();
    }

    private string[] BuildArgs(bool dryRun)
    {
        var args = new List<string> { "launch" };
        if (ServerBox.SelectedItem is string server && !string.IsNullOrWhiteSpace(server))
        {
            args.Add("--server");
            args.Add(server);
        }
        if (BattleyeBox.IsChecked == true)
        {
            args.Add("--battleye");
        }
        if (dryRun)
        {
            args.Add("--dry-run");
        }
        return [.. args];
    }

    private async Task PreviewAsync()
    {
        StatusLine.Text = "Preparing…";
        try
        {
            using var document = await _state.Cli.RunJsonAsync(BuildArgs(true));
            var root = document.RootElement;
            CommandText.Text = root.TryGetProperty("commandLine", out var line)
                ? line.GetString() ?? ""
                : "";
            StatusLine.Text = root.TryGetProperty("mods", out var mods)
                ? $"{mods.GetInt32()} mods staged"
                : "";
        }
        catch (Exception ex)
        {
            CommandText.Text = "";
            StatusLine.Text = ex.Message;
        }
    }

    private async void OnPreview(object? sender, RoutedEventArgs e) => await PreviewAsync();

    private async void OnLaunch(object? sender, RoutedEventArgs e)
    {
        LaunchButton.IsEnabled = false;
        StatusLine.Text = "Launching…";
        try
        {
            var result = await _state.Cli.RunAsync(BuildArgs(false));
            StatusLine.Text = result.Ok
                ? "Arma 3 closed."
                : $"Launch failed: {result.StdErr.Trim()}";
        }
        finally
        {
            LaunchButton.IsEnabled = true;
        }
    }
}
