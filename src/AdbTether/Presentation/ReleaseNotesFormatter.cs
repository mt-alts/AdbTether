using System.Text;
using System.Text.RegularExpressions;

namespace AdbTether.Presentation;

/// <summary>
/// Renders a GitHub release body for the update dialog. Release notes are
/// Markdown, but the dialog shows them in a single <c>TextBlock</c>, so this
/// removes the syntax that would otherwise appear literally: heading hashes,
/// bullet markers, emphasis and code ticks. It is deliberately not a Markdown
/// engine, just enough to keep the notes readable in a narrow prompt.
/// </summary>
public static class ReleaseNotesFormatter
{
    /// <summary>Matches a Markdown link and keeps its text, dropping the URL.</summary>
    private static readonly Regex LinkPattern = new(@"\[([^\]]+)\]\([^)]*\)");

    public static string Format(string? markdown, string? version = null)
    {
        if (string.IsNullOrWhiteSpace(markdown))
            return "";

        string[] lines = markdown.Replace("\r\n", "\n").Replace('\r', '\n').Split('\n');
        var kept = new List<string>(lines.Length);
        bool fenced = false;

        for (int i = SkipReleaseTitle(lines, version); i < lines.Length; i++)
        {
            string line = lines[i];

            if (IsFence(line))
            {
                // The fence delimits a code block but carries no text of its own.
                fenced = !fenced;
                continue;
            }

            // Inside a fence the text is code: keep it verbatim (indentation and
            // all) so a "#" there is not mistaken for a heading.
            kept.Add(fenced ? line.TrimEnd() : FormatProse(line));
        }

        return Join(kept);
    }

    /// <summary>
    /// Index of the first line worth keeping. A release body usually opens with
    /// its own title ("## AdbTether 0.1.1"), which only repeats the version the
    /// dialog already shows above the notes.
    /// </summary>
    private static int SkipReleaseTitle(string[] lines, string? version)
    {
        if (string.IsNullOrEmpty(version))
            return 0;

        string bare = version.TrimStart('v', 'V');

        for (int i = 0; i < lines.Length; i++)
        {
            if (lines[i].Trim().Length == 0)
                continue;

            string? heading = HeadingText(lines[i]);
            bool isTitle = heading is not null
                && heading.Contains(bare, StringComparison.OrdinalIgnoreCase);

            return isTitle ? i + 1 : 0;
        }

        return 0;
    }

    private static bool IsFence(string line) =>
        line.TrimStart().StartsWith("```", StringComparison.Ordinal);

    private static string FormatProse(string line)
    {
        string? heading = HeadingText(line);
        if (heading is not null)
            return heading; // A heading is its own block; nothing else to strip.

        return StripInline(StripBullet(line));
    }

    /// <summary>Text of a heading line, or null when the line is not one.</summary>
    private static string? HeadingText(string line)
    {
        string trimmed = line.TrimStart();
        int hashes = 0;
        while (hashes < trimmed.Length && trimmed[hashes] == '#')
            hashes++;

        // "### Fixed" is a heading; "#GUI" is a comment, not a heading.
        bool isHeading = hashes is > 0 and <= 6
            && hashes < trimmed.Length
            && trimmed[hashes] == ' ';

        return isHeading ? trimmed[(hashes + 1)..].TrimEnd() : null;
    }

    /// <summary>"- item" / "* item" -> "• item".</summary>
    private static string StripBullet(string line)
    {
        string trimmed = line.TrimStart();
        bool isBullet = trimmed.StartsWith("- ", StringComparison.Ordinal)
            || trimmed.StartsWith("* ", StringComparison.Ordinal);

        return isBullet ? "• " + trimmed[2..] : line;
    }

    private static string StripInline(string line)
    {
        line = LinkPattern.Replace(line, "$1");
        return line.Replace("**", "").Replace("__", "").Replace("`", "").TrimEnd();
    }

    /// <summary>Drops leading and repeated blank lines, which the syntax leaves behind.</summary>
    private static string Join(List<string> lines)
    {
        var builder = new StringBuilder();
        bool previousBlank = true;

        foreach (string line in lines)
        {
            bool blank = line.Length == 0;
            if (blank && previousBlank)
                continue;

            if (builder.Length > 0)
                builder.Append('\n');

            builder.Append(line);
            previousBlank = blank;
        }

        return builder.ToString().TrimEnd();
    }
}
