using AdbTether.Models;
using AdbTether.Services;
using AdbTether.ViewModels.Dialogs;
using AdbTether.Views;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace AdbTether;

/// <summary>
/// Modal dialog for pairing a device. All state and validation live in
/// <see cref="AdbPairDialogViewModel"/>; this class only wires the view.
/// </summary>
public sealed partial class AdbPairDialog : ThemedContentDialog
{
    private readonly AdbPairDialogViewModel _viewModel;

    public AdbPairDialog()
    {
        InitializeComponent();

        _viewModel = new AdbPairDialogViewModel(NetworkEndpointHelper.GetDefaultGatewayAddress() ?? "");
        DataContext = _viewModel;

        Opened += (_, _) => IpAddressTextBox.Focus(FocusState.Programmatic);
        PrimaryButtonClick += OnPrimaryButtonClick;
    }

    /// <summary>The validated request, or null if the user cancelled or validation failed.</summary>
    public AdbPairRequest? Request { get; private set; }

    private void OnPrimaryButtonClick(ContentDialog sender, ContentDialogButtonClickEventArgs args)
    {
        if (!_viewModel.TryBuildRequest(out var request))
        {
            args.Cancel = true;
            return;
        }

        Request = request;
    }
}
