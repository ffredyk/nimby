using System;
using System.Collections.Generic;
using System.Threading.Tasks;
using Avalonia.Controls;
using Avalonia.Interactivity;
using Nimby.App.Models;
using Nimby.App.Services;

namespace Nimby.App.Views;

public partial class ConfigView : UserControl, IRefreshable
{
    private readonly AppState _state;

    public ConfigView(AppState state)
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

        ArmaBox.Text = config.ArmaPath ?? config.DetectedArma ?? "";
        ArmaBox.PlaceholderText = config.DetectedArma is null
            ? "Arma 3 folder"
            : $"detected: {config.DetectedArma}";
        TempBox.Text = config.TempPath ?? "";
        TempBox.PlaceholderText = config.EffectiveTemp;
        SharedBox.Text = config.SharedPath ?? "";
        SharedBox.PlaceholderText = "disabled (downloads go straight into the workspace)";
        WorkspaceBox.Text = config.WorkspacePath;
        ParallelBox.Text = config.ParallelDownloads.ToString();
        ModParallelBox.Text = config.ParallelMods.ToString();
        BattleyeBox.IsChecked = config.Battleye;

        ModeBox.ItemsSource = config.LinkModes;
        ModeBox.SelectedItem = config.LinkMode;

        StoreSummary.Text = config.SharedEnabled
            ? $"{config.StoreMods} mods in the store · {ModRow.FormatBytes(config.StoreBytes)}"
            : "shared store disabled";
        ConfigPathText.Text = $"config: {config.ConfigPath}";
        if (!string.IsNullOrEmpty(config.EffectiveArma) && !config.ArmaValid)
        {
            StatusLine.Text = "The configured Arma folder does not contain arma3 executable.";
        }
    }

    private async void OnBrowseArma(object? sender, RoutedEventArgs e) =>
        await BrowseInto(ArmaBox, "Select the Arma 3 folder");

    private async void OnBrowseTemp(object? sender, RoutedEventArgs e) =>
        await BrowseInto(TempBox, "Select the scratch folder");

    private async void OnBrowseShared(object? sender, RoutedEventArgs e) =>
        await BrowseInto(SharedBox, "Select the shared mod store");

    private async void OnBrowseWorkspace(object? sender, RoutedEventArgs e) =>
        await BrowseInto(WorkspaceBox, "Select the workspace folder");

    private async Task BrowseInto(TextBox box, string title)
    {
        try
        {
            var path = await Dialogs.PickFolderAsync(this, title);
            if (!string.IsNullOrWhiteSpace(path))
            {
                box.Text = path;
            }
        }
        catch (Exception ex)
        {
            StatusLine.Text = ex.Message;
        }
    }

    private void OnUseDetectedArma(object? sender, RoutedEventArgs e)
    {
        var detected = _state.Config?.DetectedArma;
        if (!string.IsNullOrEmpty(detected))
        {
            ArmaBox.Text = detected;
        }
        else
        {
            StatusLine.Text = "Steam could not find Arma 3; browse to it manually.";
        }
    }

    private void OnClearTemp(object? sender, RoutedEventArgs e) => TempBox.Text = "";

    private void OnClearShared(object? sender, RoutedEventArgs e) => SharedBox.Text = "";

    private void OnClearWorkspace(object? sender, RoutedEventArgs e)
    {
        var local = Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData);
        WorkspaceBox.Text = System.IO.Path.Combine(local, "Nimby", "mods");
    }

    private async void OnSave(object? sender, RoutedEventArgs e)
    {
        var args = new List<string>();

        var arma = ArmaBox.Text?.Trim();
        if (!string.IsNullOrEmpty(arma))
        {
            args.Add("--arma");
            args.Add(arma);
        }
        else if (_state.Config?.ArmaPath is not null)
        {
            args.Add("--clear-arma");
        }

        var temp = TempBox.Text?.Trim();
        if (!string.IsNullOrEmpty(temp))
        {
            args.Add("--temp");
            args.Add(temp);
        }
        else if (_state.Config?.TempPath is not null)
        {
            args.Add("--clear-temp");
        }

        var shared = SharedBox.Text?.Trim();
        if (!string.IsNullOrEmpty(shared))
        {
            args.Add("--shared");
            args.Add(shared);
        }
        else if (_state.Config?.SharedPath is not null)
        {
            args.Add("--clear-shared");
        }

        var workspace = WorkspaceBox.Text?.Trim();
        if (!string.IsNullOrEmpty(workspace))
        {
            args.Add("--workspace");
            args.Add(workspace);
        }

        if (ModeBox.SelectedItem is string mode)
        {
            args.Add("--link-mode");
            args.Add(mode);
        }

        if (int.TryParse(ModParallelBox.Text?.Trim(), out var mods) && mods > 0)
        {
            args.Add("--mod-parallel");
            args.Add(mods.ToString());
        }
        if (int.TryParse(ParallelBox.Text?.Trim(), out var parallel) && parallel > 0)
        {
            args.Add("--parallel");
            args.Add(parallel.ToString());
        }

        args.Add("--battleye");
        args.Add(BattleyeBox.IsChecked == true ? "true" : "false");

        SaveButton.IsEnabled = false;
        StatusLine.Text = "Saving…";
        try
        {
            var result = await _state.SetConfigAsync(args);
            StatusLine.Text = result.Ok
                ? "Saved."
                : $"Save failed: {result.StdErr.Trim()}";
            await RefreshAsync();
        }
        catch (Exception ex)
        {
            StatusLine.Text = $"Save failed: {ex.Message}";
        }
        finally
        {
            SaveButton.IsEnabled = true;
        }
    }
}
