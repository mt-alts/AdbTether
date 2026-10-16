using AdbTether.Abstractions;
using AdbTether.Models;
using Microsoft.UI.Xaml.Controls;

namespace AdbTether.ViewModels;

/// <summary>Checks GitHub for a newer release and drives the update prompt.</summary>
public sealed class UpdateViewModel : ViewModelBase
{
    private readonly IUpdateService _update;
    private readonly IDialogService _dialogs;
    private readonly IUpdatePreferences _preferences;
    private bool _isChecking;

    private bool _statusOpen;
    private InfoBarSeverity _statusSeverity = InfoBarSeverity.Informational;
    private string _statusTitle = "";
    private string _statusMessage = "";

    public UpdateViewModel(IUpdateService update, IDialogService dialogs, IUpdatePreferences preferences)
    {
        _update = update;
        _dialogs = dialogs;
        _preferences = preferences;
        CheckCommand = new AsyncRelayCommand(() => CheckAsync());
    }

    public string AppVersion => AppInfo.CurrentVersion;

    public AsyncRelayCommand CheckCommand { get; }

    /// <summary>Drives the inline result InfoBar on the Settings page.</summary>
    public bool StatusOpen
    {
        get => _statusOpen;
        set => SetProperty(ref _statusOpen, value);
    }

    public InfoBarSeverity StatusSeverity
    {
        get => _statusSeverity;
        private set => SetProperty(ref _statusSeverity, value);
    }

    public string StatusTitle
    {
        get => _statusTitle;
        private set => SetProperty(ref _statusTitle, value);
    }

    public string StatusMessage
    {
        get => _statusMessage;
        private set => SetProperty(ref _statusMessage, value);
    }

    /// <summary>
    /// Checks for an update; if one exists, prompts to download it, otherwise
    /// reports "up to date" in the Settings InfoBar. A failed check is reported
    /// to the user so it is never mistaken for "up to date"; the startup check
    /// passes <paramref name="userInitiated"/> as false to stay silent when the
    /// machine is simply offline (and to avoid an InfoBar on every launch).
    /// </summary>
    public async Task CheckAsync(bool userInitiated = true)
    {
        if (_isChecking)
            return;

        // The user asked not to be prompted on launch, so skip the startup check
        // entirely. The Settings check stays available as the way back in.
        if (!userInitiated && _preferences.SkipStartupPrompt)
            return;

        _isChecking = true;

        // Clear any previous result so a stale "up to date" never sits under a
        // fresh check.
        if (userInitiated)
            StatusOpen = false;

        try
        {
            var info = await _update.CheckForUpdateAsync();
            if (info is not null)
                await _dialogs.ShowUpdateAsync(
                    info,
                    progress => _update.DownloadAndInstallAsync(info, progress),
                    offerSkipStartup: !userInitiated);
            else if (userInitiated)
                ShowUpToDate();
        }
        catch (Exception ex)
        {
            // Non-critical, but swallowing this made a broken update path look
            // identical to "up to date", so surface it for user-initiated checks.
            if (userInitiated)
                ShowError(ex.Message);
        }
        finally
        {
            _isChecking = false;
        }
    }

    private void ShowUpToDate()
    {
        StatusSeverity = InfoBarSeverity.Success;
        StatusTitle = "You're up to date";
        StatusMessage = $"AdbTether v{AppVersion} is the latest version.";
        StatusOpen = true;
    }

    private void ShowError(string detail)
    {
        StatusSeverity = InfoBarSeverity.Error;
        StatusTitle = "Update check failed";
        StatusMessage = $"AdbTether could not reach GitHub to check for a new release. {detail}";
        StatusOpen = true;
    }
}
