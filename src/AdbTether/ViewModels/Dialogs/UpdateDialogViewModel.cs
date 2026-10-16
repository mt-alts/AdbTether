using AdbTether.Models;
using Microsoft.UI.Xaml;

namespace AdbTether.ViewModels.Dialogs;

/// <summary>State for the update dialog: versions, download progress and errors.</summary>
public sealed class UpdateDialogViewModel : ViewModelBase
{
    private bool _downloading;
    private double _progress;
    private string _progressText = "";
    private string _error = "";

    public UpdateDialogViewModel(UpdateInfo info)
    {
        CurrentVersion = AppInfo.CurrentVersion;
        LatestVersion = info.LatestVersion;
    }

    public string CurrentVersion { get; }

    /// <summary>Current version with the display prefix, e.g. "v0.1.0".</summary>
    public string CurrentVersionText => $"v{CurrentVersion}";

    public string LatestVersion { get; }

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
}
