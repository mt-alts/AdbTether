using AdbTether.Models;
using AdbTether.Services;
using AdbTether.ViewModels.Dialogs;
using AdbTether.Views;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace AdbTether;

/// <summary>
/// Modal dialog for connecting to a device over TCP/IP. All state and validation
/// live in <see cref="AdbConnectDialogViewModel"/>; this class only wires the view.
/// </summary>
public sealed partial class AdbConnectDialog : ThemedContentDialog
{
    private readonly AdbConnectDialogViewModel _viewModel;

    public AdbConnectDialog()
    {
        InitializeComponent();

        _viewModel = new AdbConnectDialogViewModel(NetworkEndpointHelper.GetDefaultGatewayAddress() ?? "");
        DataContext = _viewModel;

        Opened += (_, _) => IpAddressTextBox.Focus(FocusState.Programmatic);
        PrimaryButtonClick += OnPrimaryButtonClick;
    }

    /// <summary>The validated request, or null if the user cancelled or validation failed.</summary>
    public AdbConnectRequest? Request { get; private set; }

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
