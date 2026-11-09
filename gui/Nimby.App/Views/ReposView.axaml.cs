using System;
using System.Collections.ObjectModel;
using System.Linq;
using System.Threading.Tasks;
using Avalonia.Controls;
using Avalonia.Interactivity;
using Nimby.App.Models;
using Nimby.App.Services;

namespace Nimby.App.Views;

public partial class ReposView : UserControl, IRefreshable
{
    private readonly AppState _state;
    private readonly ObservableCollection<RepoRow> _repos = [];

    public ReposView(AppState state)
    {
        _state = state;
        InitializeComponent();
        RepoList.ItemsSource = _repos;
    }

    public async Task RefreshAsync()
    {
        await _state.RefreshConfigAsync();
        var config = _state.Config;

        var selectedUrl = (RepoList.SelectedItem as RepoRow)?.Url;
        _repos.Clear();
        if (config is not null)
        {
            foreach (var repo in config.Repos)
            {
                _repos.Add(repo);
            }
        }

        var restore = _repos.FirstOrDefault(r => r.Url == selectedUrl)
                      ?? _repos.FirstOrDefault(r => r.Active)
                      ?? _repos.FirstOrDefault();
        RepoList.SelectedItem = restore;
        if (restore is not null)
        {
            await ShowDetailsAsync(restore.Url, false);
        }
        else
        {
            ClearDetails();
        }
    }

    private void ClearDetails()
    {
        DetailName.Text = "No repository selected";
        DetailUrl.Text = "";
        DetailSummary.Text = "—";
        DetailMeta.Text = "";
        DetailServers.Text = "";
        DetailParameters.Text = "";
        DetailStatus.Text = "";
        RepoPathBox.Text = "";
        RepoPathHint.Text = "";
        BannerHost.IsVisible = false;
        BannerImage.Source = null;
        IconImage.Source = null;
    }

    // ------------------------------------------------------------ install folder

    private void OnClearAddPath(object? sender, RoutedEventArgs e) => AddPathBox.Text = "";

    private void OnBrowseRepoPath(object? sender, RoutedEventArgs e) =>
        RepoPathHint.Text = "Type a folder name; it always lives under the workspace root.";

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
            DetailMessage = ex.Message;
        }
    }

    private async void OnApplyRepoPath(object? sender, RoutedEventArgs e)
    {
        try
        {
            if (RepoList.SelectedItem is not RepoRow row)
            {
                return;
            }
            var folder = RepoPathBox.Text?.Trim();
            if (string.IsNullOrWhiteSpace(folder))
            {
                DetailMessage = "Enter a folder name, or derive it from the repository name.";
                return;
            }
            var result = await _state.RepoAsync(["folder", row.Url, folder]);
            DetailMessage = result.Ok
                ? $"Installs into {System.IO.Path.Combine(_state.Config?.WorkspacePath ?? "", folder)}"
                : result.StdErr.Trim();
            await RefreshAsync();
        }
        catch (Exception ex)
        {
            DetailMessage = ex.Message;
        }
    }

    private async void OnResetRepoPath(object? sender, RoutedEventArgs e)
    {
        try
        {
            if (RepoList.SelectedItem is not RepoRow row)
            {
                return;
            }
            var result = await _state.RepoAsync(["folder", row.Url, "--default"]);
            DetailMessage = result.Ok
                ? "Folder derived from the repository name."
                : result.StdErr.Trim();
            await RefreshAsync();
        }
        catch (Exception ex)
        {
            DetailMessage = ex.Message;
        }
    }

    private async void OnSelectionChanged(object? sender, SelectionChangedEventArgs e)
    {
        try
        {
            if (RepoList.SelectedItem is RepoRow row)
            {
                await ShowDetailsAsync(row.Url, false);
            }
        }
        catch (Exception ex)
        {
            DetailMessage = ex.Message;
        }
    }

    private async Task ShowDetailsAsync(string url, bool force)
    {
        DetailName.Text = url;
        DetailUrl.Text = url;
        DetailStatus.Text = "Loading…";
        var row = _repos.FirstOrDefault(r => r.Url == url);
        RepoPathBox.Text = row?.Folder ?? "";
        RepoPathHint.Text = row is null
            ? ""
            : $"installs into {row.Path}";
        var snapshot = await _state.SnapshotAsync(url, force);
        if (snapshot is null)
        {
            DetailSummary.Text = "unreachable";
            DetailStatus.Text = "Could not read repo.json from this URL.";
            BannerHost.IsVisible = false;
            IconImage.Source = null;
            return;
        }

        DetailName.Text = snapshot.Name;
        DetailSummary.Text = snapshot.Summary;
        DetailMeta.Text = snapshot.Mods.Count == 0 ? "" : $"{snapshot.Mods.Count} mods";
        DetailServers.Text = snapshot.Servers.Count == 0
            ? "No servers published."
            : $"Servers: {string.Join(", ", snapshot.Servers)}";
        DetailParameters.Text = string.IsNullOrWhiteSpace(snapshot.ClientParameters)
            ? ""
            : $"client: {snapshot.ClientParameters}";
        DetailStatus.Text = "";

        IconImage.Source = Images.TryLoad(snapshot.IconPath);
        var banner = Images.TryLoad(snapshot.BannerPath);
        BannerImage.Source = banner;
        BannerHost.IsVisible = banner is not null;
    }

    private string DetailMessage
    {
        set => DetailStatus.Text = value;
    }

    private async void OnAdd(object? sender, RoutedEventArgs e)
    {
        try
        {
            var url = UrlBox.Text?.Trim();
            if (string.IsNullOrWhiteSpace(url))
            {
                DetailStatus.Text = "Enter a repository URL first.";
                return;
            }

            var list = new System.Collections.Generic.List<string> { "add", url };
            if (!string.IsNullOrWhiteSpace(NameBox.Text))
            {
                list.Add("--name");
                list.Add(NameBox.Text.Trim());
            }
            if (!string.IsNullOrWhiteSpace(AddPathBox.Text))
            {
                list.Add("--folder");
                list.Add(AddPathBox.Text.Trim());
            }
            list.Add("--select");

            var result = await _state.RepoAsync(list);
            if (!result.Ok)
            {
                DetailMessage = result.StdErr.Trim();
                return;
            }

            UrlBox.Text = "";
            NameBox.Text = "";
            AddPathBox.Text = "";
            await RefreshAsync();
        }
        catch (Exception ex)
        {
            DetailMessage = ex.Message;
        }
    }

    private async void OnSelect(object? sender, RoutedEventArgs e)
    {
        try
        {
            if (RepoList.SelectedItem is not RepoRow row)
            {
                return;
            }
            await _state.RepoAsync(["select", row.Url]);
            await RefreshAsync();
        }
        catch (Exception ex)
        {
            DetailMessage = ex.Message;
        }
    }

    private async void OnRemove(object? sender, RoutedEventArgs e)
    {
        try
        {
            if (RepoList.SelectedItem is not RepoRow row)
            {
                return;
            }
            var result = await _state.RepoAsync(["remove", row.Url]);
            if (!result.Ok)
            {
                DetailMessage = result.StdErr.Trim();
            }
            await RefreshAsync();
        }
        catch (Exception ex)
        {
            DetailMessage = ex.Message;
        }
    }

    private async void OnReload(object? sender, RoutedEventArgs e)
    {
        try
        {
            if (RepoList.SelectedItem is RepoRow row)
            {
                _state.ForgetSnapshots();
                await ShowDetailsAsync(row.Url, true);
            }
        }
        catch (Exception ex)
        {
            DetailMessage = ex.Message;
        }
    }
}
