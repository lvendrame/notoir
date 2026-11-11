param([string]$Directory, [switch]$CompileOnly)
$ErrorActionPreference = 'Stop'
try {
    Add-Type -AssemblyName System.Windows.Forms,System.Drawing
    Add-Type -ReferencedAssemblies System.Windows.Forms,System.Drawing -TypeDefinition @'
using System;
using System.IO;
using System.Drawing;
using System.Drawing.Printing;
using System.Windows.Forms;
public static class NotoirPrint {
    public static int Run(string directory) {
        string text = File.ReadAllText(Path.Combine(directory, "text"));
        string selectionPath = Path.Combine(directory, "selection");
        string selection = File.Exists(selectionPath) ? File.ReadAllText(selectionPath) : null;
        using (var document = new PrintDocument())
        using (var dialog = new PrintDialog())
        using (var font = new Font(FontFamily.GenericMonospace, 10))
        using (var format = new StringFormat(StringFormat.GenericTypographic)) {
            document.DocumentName = File.ReadAllText(Path.Combine(directory, "title"));
            dialog.Document = document;
            dialog.UseEXDialog = true;
            dialog.AllowSelection = selection != null;
            dialog.AllowSomePages = true;
            document.PrinterSettings.MinimumPage = 1;
            document.PrinterSettings.MaximumPage = 999999;
            document.PrinterSettings.FromPage = 1;
            document.PrinterSettings.ToPage = 999999;
            if (dialog.ShowDialog() != DialogResult.OK) return 2;
            string content = document.PrinterSettings.PrintRange == PrintRange.Selection ? selection : text;
            int offset = 0, page = 1;
            bool cancelled = false;
            format.FormatFlags |= StringFormatFlags.LineLimit;
            document.PrintController = new StandardPrintController();
            document.PrintPage += delegate(object sender, PrintPageEventArgs e) {
                // PrintDocument does not implement page ranges for the caller.
                // Paginate skipped pages against the actual selected printer.
                while (true) {
                    string remaining = content.Substring(offset);
                    int characters, lines;
                    e.Graphics.MeasureString(remaining, font, e.MarginBounds.Size, format, out characters, out lines);
                    if (remaining.Length > 0 && characters == 0)
                        throw new InvalidOperationException("The printable area is too small for a line of text.");
                    bool inRange = document.PrinterSettings.PrintRange != PrintRange.SomePages ||
                        (page >= document.PrinterSettings.FromPage && page <= document.PrinterSettings.ToPage);
                    if (inRange) e.Graphics.DrawString(remaining.Substring(0, characters), font,
                        Brushes.Black, e.MarginBounds, format);
                    offset += characters;
                    page++;
                    bool more = offset < content.Length &&
                        (document.PrinterSettings.PrintRange != PrintRange.SomePages || page <= document.PrinterSettings.ToPage);
                    if (!inRange && !more)
                        throw new InvalidOperationException("The requested page range is outside the document.");
                    if (inRange) { e.HasMorePages = more; return; }
                }
            };
            document.EndPrint += delegate(object sender, PrintEventArgs e) { cancelled = e.Cancel; };
            document.Print();
            return cancelled ? 2 : 0;
        }
    }
}
'@
    if ($CompileOnly) { exit 0 }
    if (-not $Directory) { throw 'Missing print snapshot directory.' }
    exit ([NotoirPrint]::Run($Directory))
} catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 1
}
