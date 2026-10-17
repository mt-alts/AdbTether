using AdbTether.Abstractions;
using AdbTether.Models;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml.Controls;

namespace AdbTether.ViewModels;

/// <summary>Checks GitHub for a newer release and drives the update prompt.</summary>
public sealed class UpdateViewModel : ViewModelBase
{
    /// <summary>How long a finished result stays up before it closes itself.</summary>
    private static readonly TimeSpan SnackLifetime = TimeSpan.FromSeconds(3);

    private readonly IUpdateService _update;
    private readonly IDialogService _dialogs;
    private readonly IUpdatePreferences _preferences;
    private readonly DispatcherQueueTimer _snackTimer;
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
        CheckCommand = new AsyncRelayCommand(() => CheckAsync(), () => !IsChecking);

        // A finished result closes itself, so it reads as a snackbar rather than
        // a bar that sits there until it is dismissed.
        _snackTimer = DispatcherQueue.GetForCurrentThread().CreateTimer();
        _snackTimer.Interval = SnackLifetime;
        _snackTimer.IsRepeating = false;
        _snackTimer.Tick += (_, _) => StatusOpen = false;
    }

    public string AppVersion => AppInfo.CurrentVersion;

    public AsyncRelayCommand CheckCommand { get; }

    /// <summary>
    /// True while a check runs, the silent startup one included. The Settings
    /// button greys out for it, so a click on a busy check shows as a disabled
    /// button instead of being dropped without a trace.
    /// </summary>
    public bool IsChecking
    {
        get => _isChecking;
        private set
        {
            if (SetProperty(ref _isChecking, value))
                CheckCommand.RaiseCanExecuteChanged();
        }
    }

    /// <summary>
    /// Whether the app looks for a release on launch, as the Settings switch
    /// shows it. It reads and writes the stored preference directly, so the
    /// switch and the update prompt's own checkbox always agree.
    /// </summary>
    public bool CheckAtStartup
    {
        get => !_preferences.SkipStartupPrompt;
        set
        {
            if (value == !_preferences.SkipStartupPrompt)
                return;

            _preferences.SkipStartupPrompt = !value;
            OnPropertyChanged();
        }
    }

    /// <summary>Drives the result snackbar on the Settings page.</summary>
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
    /// reports "up to date" in the Settings InfoBar. A user-initiated check
    /// announces itself in that InfoBar while it runs, so the button never looks
    /// dead on a slow link. A failed check is reported to the user so it is never
    /// mistaken for "up to date"; the startup check passes
    /// <paramref name="userInitiated"/> as false to stay silent when the machine
    /// is simply offline (and to avoid an InfoBar on every launch).
    /// </summary>
    public async Task CheckAsync(bool userInitiated = true)
    {
        if (IsChecking)
            return;

        // The user asked not to be prompted on launch, so skip the startup check
        // entirely. The Settings check stays available as the way back in.
        if (!userInitiated && _preferences.SkipStartupPrompt)
            return;

        IsChecking = true;

        try
        {
            // A check can take a moment on a slow link, so say what is happening
            // rather than leaving the click looking like it did nothing. It also
            // replaces any previous result, so a stale "up to date" never sits
            // under a fresh check.
            if (userInitiated)
                ShowChecking();

            var info = await _update.CheckForUpdateAsync();
            if (info is not null)
            {
                // The prompt is the feedback for this check, so the snackbar that
                // announced it must not sit there behind the dialog.
                StatusOpen = false;

                await _dialogs.ShowUpdateAsync(
                    info,
                    progress => _update.DownloadAndInstallAsync(info, progress),
                    offerSkipStartup: !userInitiated);

                // The prompt's checkbox writes the same preference, so re-read it
                // for the Settings switch the prompt was shown over.
                OnPropertyChanged(nameof(CheckAtStartup));
            }
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
            IsChecking = false;
        }
    }

    private void ShowChecking()
    {
        // The check is still running, so this one must not close itself.
        _snackTimer.Stop();

        StatusSeverity = InfoBarSeverity.Informational;
        StatusTitle = "Checking for updates";
        StatusMessage = "Looking for a new release on GitHub...";
        StatusOpen = true;
    }

    private void ShowUpToDate() =>
        ShowResult(InfoBarSeverity.Success, "You're up to date",
            $"AdbTether v{AppVersion} is the latest version.");

    private void ShowError(string detail) =>
        ShowResult(InfoBarSeverity.Error, "Update check failed",
            $"AdbTether could not reach GitHub to check for a new release. {detail}");

    /// <summary>Shows a finished result, which then closes itself.</summary>
    private void ShowResult(InfoBarSeverity severity, string title, string message)
    {
        StatusSeverity = severity;
        StatusTitle = title;
        StatusMessage = message;
        StatusOpen = true;

        _snackTimer.Stop();
        _snackTimer.Start();
    }
}
