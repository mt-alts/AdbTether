using System.Net;
using AdbTether.Models;

namespace AdbTether.ViewModels.Dialogs;

/// <summary>Validation and state for the pairing dialog.</summary>
public sealed class AdbPairDialogViewModel : ViewModelBase
{
    private string _ipAddress;
    private string _port = "";
    private string _pairingCode = "";
    private string _errorMessage = "";

    public AdbPairDialogViewModel(string defaultAddress) => _ipAddress = defaultAddress;

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

    public string PairingCode
    {
        get => _pairingCode;
        set => SetProperty(ref _pairingCode, value);
    }

    public string ErrorMessage
    {
        get => _errorMessage;
        set => SetProperty(ref _errorMessage, value);
    }

    /// <summary>Validates the fields and produces the request, or reports an error.</summary>
    public bool TryBuildRequest(out AdbPairRequest? request)
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

        if (string.IsNullOrWhiteSpace(PairingCode))
        {
            ErrorMessage = "Pairing code is required.";
            return false;
        }

        request = new AdbPairRequest(ip, port, PairingCode.Trim());
        return true;
    }
}
