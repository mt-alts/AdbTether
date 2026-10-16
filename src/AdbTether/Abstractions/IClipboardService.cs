namespace AdbTether.Abstractions;

/// <summary>System clipboard, abstracted so view models stay UI-free and testable.</summary>
public interface IClipboardService
{
    void SetText(string text);
}
