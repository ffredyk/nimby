namespace Nimby.App.Views;

/// <summary>Pages implement this so the window can refresh them on tab change.</summary>
public interface IRefreshable
{
    System.Threading.Tasks.Task RefreshAsync();
}
