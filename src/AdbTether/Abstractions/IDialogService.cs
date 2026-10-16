using AdbTether.Models;
using Microsoft.UI.Xaml;

namespace AdbTether.Abstractions;

/// <summary>
/// Shows modal dialogs on behalf of view models, so no view model has to know
/// how a <c>ContentDialog</c> is built or parented.
/// </summary>
public interface IDialogService
{
    /// <summary>Supplies the XamlRoot the dialogs are parented to. Call once the shell is loaded.</summary>
    void Initialize(XamlRoot xamlRoot);

    /// <summary>Returns the validated connect request, or null if the user cancelled.</summary>
    Task<AdbConnectRequest?> ShowAdbConnectAsync();

    /// <summary>Returns the validated pairing request, or null if the user cancelled.</summary>
    Task<AdbPairRequest?> ShowAdbPairAsync();

    /// <summary>
    /// Warns that the background service is unreachable, listing the commands to
    /// start or install it. Returns true when the user asks to retry, in which
    /// case the caller should re-check availability.
    /// </summary>
    Task<bool> ShowServiceOfflineAsync(string detail);

    /// <summary>
    /// Shows the update prompt and, if accepted, runs <paramref name="download"/>
    /// while reporting progress to the open dialog. When
    /// <paramref name="offerSkipStartup"/> is set the prompt also offers to stop
    /// checking for updates at startup.
    /// </summary>
    Task ShowUpdateAsync(UpdateInfo info, Func<IProgress<double>, Task> download, bool offerSkipStartup);
}
