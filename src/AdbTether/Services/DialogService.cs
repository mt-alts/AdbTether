using AdbTether.Abstractions;
using AdbTether.Models;
using AdbTether.ViewModels.Dialogs;
using AdbTether.Views;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace AdbTether.Services;

/// <summary>
/// Builds and shows the app's modal dialogs. This is the only place that knows
/// how a dialog is parented and how the update download is driven.
/// </summary>
public sealed class DialogService : IDialogService
{
    private XamlRoot? _root;

    public void Initialize(XamlRoot xamlRoot) => _root = xamlRoot;

    public async Task<AdbConnectRequest?> ShowAdbConnectAsync()
    {
        if (_root is null)
            return null;

        var dialog = new AdbConnectDialog { XamlRoot = _root };
        return await dialog.ShowAsync() == ContentDialogResult.Primary ? dialog.Request : null;
    }

    public async Task<AdbPairRequest?> ShowAdbPairAsync()
    {
        if (_root is null)
            return null;

        var dialog = new AdbPairDialog { XamlRoot = _root };
        return await dialog.ShowAsync() == ContentDialogResult.Primary ? dialog.Request : null;
    }

    public async Task<bool> ShowServiceOfflineAsync(string detail)
    {
        if (_root is null)
            return false;

        var dialog = new ServiceOfflineDialog(detail) { XamlRoot = _root };
        return await dialog.ShowAsync() == ContentDialogResult.Primary;
    }

    public async Task ShowUpdateAsync(UpdateInfo info, Func<IProgress<double>, Task> download)
    {
        if (_root is null)
            return;

        var viewModel = new UpdateDialogViewModel(info);
        var dialog = new UpdateDialog(viewModel) { XamlRoot = _root };
        bool started = false;

        dialog.PrimaryButtonClick += async (_, args) =>
        {
            // Keep the dialog open while the download runs so progress is visible.
            args.Cancel = true;
            if (started)
                return;
            started = true;

            viewModel.BeginDownload();
            try
            {
                await download(new Progress<double>(viewModel.ReportProgress));
            }
            catch (Exception ex)
            {
                viewModel.ShowError($"Download failed: {ex.Message}");
                return;
            }

            dialog.Hide();
        };

        await dialog.ShowAsync();
    }
}
