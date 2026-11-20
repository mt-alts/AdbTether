using Microsoft.UI.Xaml.Controls;
using AdbTether.Models;

namespace AdbTether.Pages;

public sealed partial class AboutPage : Page
{
    public AboutPage()
    {
        InitializeComponent();
        VersionText.Text = AppInfo.CurrentVersion;
        GitHubLink.NavigateUri = new Uri(AppInfo.RepositoryUrl);
    }
}
