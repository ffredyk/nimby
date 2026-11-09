using System;
using System.Threading.Tasks;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Media.Imaging;
using Avalonia.Platform.Storage;
using Avalonia.VisualTree;

namespace Nimby.App.Services;

public static class Dialogs
{
    /// <summary>Folder picker bound to the window the control lives in.</summary>
    public static async Task<string?> PickFolderAsync(Visual anchor, string title)
    {
        var top = TopLevel.GetTopLevel(anchor);
        if (top is null)
        {
            return null;
        }
        var folders = await top.StorageProvider.OpenFolderPickerAsync(new FolderPickerOpenOptions
        {
            Title = title,
            AllowMultiple = false,
        });
        return folders.Count > 0 ? folders[0].TryGetLocalPath() : null;
    }
}

public static class Images
{
    /// <summary>Load a bitmap, tolerating missing or broken files.</summary>
    public static Bitmap? TryLoad(string? path)
    {
        if (string.IsNullOrWhiteSpace(path))
        {
            return null;
        }
        try
        {
            return new Bitmap(path);
        }
        catch (Exception)
        {
            return null;
        }
    }
}
