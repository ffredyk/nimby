using System.Collections.ObjectModel;
using System.Threading.Tasks;
using Avalonia.Controls;
using Avalonia.Interactivity;
using Nimby.App.Models;
using Nimby.App.Services;

namespace Nimby.App.Views;

public partial class StatsView : UserControl, IRefreshable
{
    private readonly AppState _state;
    private readonly ObservableCollection<RepoStatsRow> _rows = [];

    public StatsView(AppState state)
    {
        _state = state;
        InitializeComponent();
        RepoList.ItemsSource = _rows;
    }

    public async Task RefreshAsync()
    {
        StatusLine.Text = "Reading workspace…";
        var stats = await _state.StatsAsync();
        if (stats is null)
        {
            StatusLine.Text = "Cannot read workspace statistics.";
            return;
        }

        LogicalText.Text = stats.LogicalText;
        OnDiskText.Text = stats.OnDiskText;
        InWorkspaceText.Text = stats.InWorkspaceText;
        DownloadedText.Text = stats.DownloadedText;
        SavedText.Text = stats.SavedText;
        SavedDetail.Text = stats.TotalSaved > 0
            ? "linked, not written again"
            : "nothing saved yet";
        WorkspaceText.Text = $"workspace: {stats.WorkspaceText}";
        StoreText.Text = $"shared store: {stats.StoreText}";
        TrackingText.Text = stats.TrackingText;

        _rows.Clear();
        foreach (var row in stats.Repos)
        {
            _rows.Add(row);
        }
        StatusLine.Text = stats.Repos.Count == 0
            ? "No mods installed yet. Use the Install tab to add some."
            : $"{stats.Repos.Count} repositor{(stats.Repos.Count == 1 ? "y" : "ies")} · refreshed";
    }

    private async void OnRefresh(object? sender, RoutedEventArgs e) => await RefreshAsync();
}
