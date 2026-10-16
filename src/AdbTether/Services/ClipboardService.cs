using AdbTether.Abstractions;
using Windows.ApplicationModel.DataTransfer;

namespace AdbTether.Services;

/// <summary>Wraps the WinRT clipboard so view models depend on an abstraction.</summary>
public sealed class ClipboardService : IClipboardService
{
    public void SetText(string text)
    {
        var package = new DataPackage();
        package.SetText(text);
        Clipboard.SetContent(package);
    }
}
