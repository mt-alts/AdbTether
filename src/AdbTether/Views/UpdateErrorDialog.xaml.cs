namespace AdbTether.Views;

/// <summary>
/// Reports that the update check could not complete, so a failure is never
/// mistaken for "you are up to date".
/// </summary>
public sealed partial class UpdateErrorDialog : ThemedContentDialog
{
    public UpdateErrorDialog(string detail)
    {
        InitializeComponent();
        DetailText.Text = detail;
    }
}
