using System.Net;
using AdbTether.Models;

namespace AdbTether.ViewModels.Dialogs;

/// <summary>Validation and state for the TCP/IP connect dialog.</summary>
public sealed class AdbConnectDialogViewModel : ViewModelBase
{
    private string _ipAddress;
    private string _port = "";
    private string _errorMessage = "";

    public AdbConnectDialogViewModel(string defaultAddress) => _ipAddress = defaultAddress;

    public string IpAddress
    {
        get => _ipAddress;
        set => SetProperty(ref _ipAddress, value);
    }

    public string Port
    {
        get => _port;
        set => SetProperty(ref _port, value);
    }

    public string ErrorMessage
    {
        get => _errorMessage;
        set => SetProperty(ref _errorMessage, value);
    }

    /// <summary>Validates the fields and produces the request, or reports an error.</summary>
    public bool TryBuildRequest(out AdbConnectRequest? request)
    {
        request = null;
        ErrorMessage = "";

        string ip = IpAddress.Trim();
        if (!IPAddress.TryParse(ip, out _))
        {
            ErrorMessage = "Enter a valid IP address.";
            return false;
        }

        if (!int.TryParse(Port.Trim(), out int port) || port is < 1 or > 65535)
        {
            ErrorMessage = "Port must be between 1 and 65535.";
            return false;
        }

        request = new AdbConnectRequest(ip, port);
        return true;
    }
}
