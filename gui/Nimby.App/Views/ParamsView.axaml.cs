using System;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.Linq;
using System.Threading.Tasks;
using Avalonia.Controls;
using Avalonia.Interactivity;
using Nimby.App.Models;
using Nimby.App.Services;

namespace Nimby.App.Views;

public partial class ParamsView : UserControl, IRefreshable
{
    private readonly AppState _state;
    private readonly ObservableCollection<ParamRow> _rows = [];
    private List<KnownParam> _known = [];

    public ParamsView(AppState state)
    {
        _state = state;
        InitializeComponent();
        ParamList.ItemsSource = _rows;
        _rows.CollectionChanged += (_, _) => UpdatePreview();
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

        _rows.Clear();
        foreach (var row in config.Params)
        {
            _rows.Add(row);
        }
        _known = config.KnownParams;
        KnownBox.ItemsSource = _known;
        if (_known.Count > 0)
        {
            KnownBox.SelectedIndex = 0;
        }
        EmptyHint.IsVisible = _rows.Count == 0;
        StatusLine.Text = "";
        UpdatePreview();
    }

    private void UpdatePreview()
    {
        var rendered = _rows
            .Where(row => row.Enabled)
            .Select(row => Render(row))
            .Where(text => !string.IsNullOrWhiteSpace(text));
        var text = string.Join(' ', rendered);
        PreviewText.Text = string.IsNullOrEmpty(text)
            ? "(no parameters)"
            : text;
    }

    /// <summary>Mirror of the CLI rendering rules.</summary>
    private static string Render(ParamRow row)
    {
        var key = row.Key.Trim();
        if (key.Length == 0)
        {
            return "";
        }
        var value = row.RawValue?.Trim();
        if (string.IsNullOrEmpty(value))
        {
            return key;
        }
        var quote = key.Equals("-name", StringComparison.OrdinalIgnoreCase)
                    || key.Equals("-profiles", StringComparison.OrdinalIgnoreCase)
                    || value.Contains(' ');
        return quote ? $"{key}=\"{value}\"" : $"{key}={value}";
    }

    private void OnKnownChanged(object? sender, SelectionChangedEventArgs e)
    {
        if (KnownBox.SelectedItem is not KnownParam spec)
        {
            return;
        }
        KnownHint.Text = spec.Description;
        KnownValueBox.IsEnabled = spec.Kind != "flag";
        KnownValueBox.PlaceholderText = spec.Kind == "flag" ? "(no value)" : "value";
        ChoicesBox.IsVisible = spec.Choices.Count > 0;
        ChoicesBox.ItemsSource = spec.Choices;
        if (spec.Choices.Count > 0)
        {
            ChoicesBox.SelectedIndex = 0;
            KnownValueBox.Text = spec.Choices[0];
        }
        else
        {
            KnownValueBox.Text = "";
        }
    }

    private void OnAddKnown(object? sender, RoutedEventArgs e)
    {
        if (KnownBox.SelectedItem is not KnownParam spec)
        {
            return;
        }
        var value = spec.Choices.Count > 0
            ? ChoicesBox.SelectedItem as string
            : KnownValueBox.Text?.Trim();
        if (spec.Kind != "flag" && string.IsNullOrWhiteSpace(value))
        {
            StatusLine.Text = $"{spec.Key} needs a value.";
            return;
        }
        Upsert(spec.Key, spec.Kind == "flag" ? null : value);
        KnownValueBox.Text = "";
        StatusLine.Text = $"Added {spec.Key}.";
        EmptyHint.IsVisible = false;
    }

    private void OnAddCustom(object? sender, RoutedEventArgs e)
    {
        var raw = CustomBox.Text?.Trim();
        if (string.IsNullOrWhiteSpace(raw))
        {
            return;
        }
        var (key, value) = Split(raw);
        if (!key.StartsWith('-'))
        {
            StatusLine.Text = "Parameters must start with '-'.";
            return;
        }
        Upsert(key, value);
        CustomBox.Text = "";
        StatusLine.Text = $"Added {key}.";
        EmptyHint.IsVisible = false;
    }

    private static (string Key, string? Value) Split(string raw)
    {
        var index = raw.IndexOf('=');
        if (index < 0)
        {
            return (raw.Trim(), null);
        }
        var key = raw[..index].Trim();
        var value = raw[(index + 1)..].Trim().Trim('"');
        return (key, string.IsNullOrEmpty(value) ? null : value);
    }

    private void Upsert(string key, string? value)
    {
        var existing = _rows.FirstOrDefault(row => row.Key.Equals(key, StringComparison.OrdinalIgnoreCase));
        var spec = _known.FirstOrDefault(k => k.Key.Equals(key, StringComparison.OrdinalIgnoreCase));
        var row = new ParamRow
        {
            Key = key,
            RawValue = value,
            Label = spec?.Label ?? "",
            Kind = spec?.Kind ?? (value is null ? "flag" : "text"),
            Display = value is null ? key : $"{key}={value}",
            Known = spec is not null,
        };
        if (existing is not null)
        {
            var index = _rows.IndexOf(existing);
            _rows[index] = row;
        }
        else
        {
            _rows.Add(row);
        }
        UpdatePreview();
    }

    private void OnRemoveParam(object? sender, RoutedEventArgs e)
    {
        if ((sender as Button)?.DataContext is not ParamRow row)
        {
            return;
        }
        _rows.Remove(row);
        EmptyHint.IsVisible = _rows.Count == 0;
        UpdatePreview();
    }

    private async void OnSave(object? sender, RoutedEventArgs e)
    {
        var args = new List<string> { "--clear-params" };
        foreach (var row in _rows.Where(r => r.Enabled))
        {
            var rendered = Render(row);
            if (string.IsNullOrWhiteSpace(rendered))
            {
                continue;
            }
            args.Add("--param");
            args.Add(rendered);
        }

        SaveButton.IsEnabled = false;
        StatusLine.Text = "Saving…";
        try
        {
            var result = await _state.SetConfigAsync(args);
            StatusLine.Text = result.Ok ? "Saved." : $"Save failed: {result.StdErr.Trim()}";
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
