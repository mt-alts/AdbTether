using AdbTether.Abstractions;
using AdbTether.Models;
using AdbTether.Presentation;
using Microsoft.UI.Xaml;

namespace AdbTether.ViewModels.Dialogs;

/// <summary>State for the update dialog: versions, download progress and errors.</summary>
public sealed class UpdateDialogViewModel : ViewModelBase
{
    /// <summary>
    /// The preference to write through to. Null when the prompt was opened from
    /// the Settings page, where the "don't check at startup" offer is hidden.
    /// </summary>
    private readonly IUpdatePreferences? _preferences;

    private bool _downloading;
    private double _progress;
    private string _progressText = "";
    private string _error = "";
    private bool _skipStartupPrompt;

    public UpdateDialogViewModel(UpdateInfo info, IUpdatePreferences preferences, bool offerSkipStartup)
    {
        CurrentVersion = AppInfo.CurrentVersion;
        LatestVersion = info.LatestVersion;
        DownloadSizeText = FormatSize(info.FileSize);
        ReleaseNotes = ReleaseNotesFormatter.Format(info.ReleaseNotes, info.LatestVersion);

        _preferences = offerSkipStartup ? preferences : null;
        _skipStartupPrompt = _preferences?.SkipStartupPrompt ?? false;
    }

    public string CurrentVersion { get; }

    /// <summary>Current version with the display prefix, e.g. "v0.1.0".</summary>
    public string CurrentVersionText => $"v{CurrentVersion}";

    public string LatestVersion { get; }

    /// <summary>Installer size, e.g. "10.7 MB"; empty when the release omits it.</summary>
    public string DownloadSizeText { get; }

    public Visibility DownloadSizeVisibility =>
        string.IsNullOrEmpty(DownloadSizeText) ? Visibility.Collapsed : Visibility.Visible;

    /// <summary>Release notes as plain text; empty when the release has none.</summary>
    public string ReleaseNotes { get; }

    public Visibility ReleaseNotesVisibility =>
        string.IsNullOrEmpty(ReleaseNotes) ? Visibility.Collapsed : Visibility.Visible;

    /// <summary>Only the startup prompt offers to stop checking on launch.</summary>
    public Visibility SkipOptionVisibility =>
        _preferences is null ? Visibility.Collapsed : Visibility.Visible;

    /// <summary>
    /// Bound two-way to the checkbox. Writes straight through to the preference,
    /// so the choice survives even when installing restarts the app.
    /// </summary>
    public bool SkipStartupPrompt
    {
        get => _skipStartupPrompt;
        set
        {
            if (!SetProperty(ref _skipStartupPrompt, value))
                return;

            if (_preferences is not null)
                _preferences.SkipStartupPrompt = value;
        }
    }

    public double Progress
    {
        get => _progress;
        private set => SetProperty(ref _progress, value);
    }

    public string ProgressText
    {
        get => _progressText;
        private set => SetProperty(ref _progressText, value);
    }

    public string Error
    {
        get => _error;
        private set
        {
            if (SetProperty(ref _error, value))
                OnPropertyChanged(nameof(ErrorVisibility));
        }
    }

    public Visibility ProgressVisibility => _downloading ? Visibility.Visible : Visibility.Collapsed;

    public Visibility ErrorVisibility => string.IsNullOrEmpty(_error) ? Visibility.Collapsed : Visibility.Visible;

    public void BeginDownload()
    {
        _downloading = true;
        ProgressText = "Downloading update...";
        OnPropertyChanged(nameof(ProgressVisibility));
    }

    public void ReportProgress(double value)
    {
        Progress = value;
        ProgressText = $"Downloading... {value:0}%";
    }

    public void ShowError(string message) => Error = message;

    // Invariant culture: the UI is English, so a Turkish locale must not turn
    // the decimal point into a comma.
    private static string FormatSize(long bytes) =>
        bytes <= 0 ? "" : FormattableString.Invariant($"{bytes / 1024.0 / 1024.0:0.#} MB");
}
