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
        CheckCommand = new AsyncRelayCommand(() => CheckAsync());
    }

    public string AppVersion => AppInfo.CurrentVersion;

    public AsyncRelayCommand CheckCommand { get; }

    /// <summary>
    /// Checks for an update; if one exists, prompts to download it. A failed
    /// check is reported to the user so it is never mistaken for "up to date";
    /// the startup check passes <paramref name="reportErrors"/> as false to stay
    /// silent when the machine is simply offline.
    /// </summary>
    public async Task CheckAsync(bool reportErrors = true)
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
        catch (Exception ex)
        {
            // Non-critical, but swallowing this made a broken update path look
            // identical to "up to date", so surface it for user-initiated checks.
            if (reportErrors)
                await _dialogs.ShowUpdateErrorAsync(ex.Message);
        }
        finally
        {
            _isChecking = false;
        }
    }
}
