using AdbTether.ViewModels.Dialogs;

namespace AdbTether.Views;

/// <summary>
/// Modal update prompt. It is a thin view over <see cref="UpdateDialogViewModel"/>;
/// the download itself is driven by the dialog service that created it.
/// </summary>
public sealed partial class UpdateDialog : ThemedContentDialog
{
    public UpdateDialog(UpdateDialogViewModel viewModel)
    {
        InitializeComponent();
        DataContext = viewModel;
    }
}
