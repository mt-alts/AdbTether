using AdbTether.Abstractions;
using AdbTether.Models;

namespace AdbTether.ViewModels;

/// <summary>Checks GitHub for a newer release and drives the update prompt.</summary>
public sealed class UpdateViewModel : ViewModelBase
{
    private readonly IUpdateService _update;
    private readonly IDialogService _dialogs;
    private bool _isChecking;

    public UpdateViewModel(IUpdateService update, IDialogService dialogs)
    {
        _update = update;
        _dialogs = dialogs;
        CheckCommand = new AsyncRelayCommand(CheckAsync);
    }

    public string AppVersion => AppInfo.CurrentVersion;

    public AsyncRelayCommand CheckCommand { get; }

    /// <summary>Checks for an update; if one exists, prompts to download it.</summary>
    public async Task CheckAsync()
    {
        if (_isChecking)
            return;

        _isChecking = true;
        try
        {
            var info = await _update.CheckForUpdateAsync();
            if (info is not null)
                await _dialogs.ShowUpdateAsync(info, progress => _update.DownloadAndInstallAsync(info, progress));
        }
        catch
        {
            // Update check is non-critical; ignore failures silently.
        }
        finally
        {
            _isChecking = false;
        }
    }
}
