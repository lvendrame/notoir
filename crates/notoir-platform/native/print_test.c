// Run under a desktop display or xvfb-run; no print dialog or printer is used.
#include "print.c"

static guint export_pdf(PrintData *data, const gchar *path, gboolean selection) {
    GtkPrintOperation *operation = gtk_print_operation_new();
    GtkPrintSettings *settings = gtk_print_settings_new();
    gtk_print_settings_set_print_pages(settings, selection ? GTK_PRINT_PAGES_SELECTION : GTK_PRINT_PAGES_ALL);
    gtk_print_operation_set_print_settings(operation, settings);
    gtk_print_operation_set_unit(operation, GTK_UNIT_POINTS);
    gtk_print_operation_set_export_filename(operation, path);
    g_signal_connect(operation, "begin-print", G_CALLBACK(begin_print), data);
    g_signal_connect(operation, "draw-page", G_CALLBACK(draw_page), data);
    GError *error = NULL;
    GtkPrintOperationResult result = gtk_print_operation_run(operation, GTK_PRINT_OPERATION_ACTION_EXPORT, NULL, &error);
    if (error) { g_printerr("%s\n", error->message); exit(1); }
    if (result != GTK_PRINT_OPERATION_RESULT_APPLY) exit(2);
    const gchar *expected = selection ? data->selection : data->text;
    if (strcmp(pango_layout_get_text(data->layout), expected) != 0) exit(3);
    return data->starts->len;
}
int main(int argc, char **argv) {
    gtk_init(&argc, &argv);
    if (argc != 2) return 4;
    PrintData full = {0};
    GString *text = g_string_new("");
    for (int i = 0; i < 150; i++) g_string_append_printf(text,
        "    日本語 line %d with a very long line that must wrap within the printable width and keep its final marker END-%d\n\n", i, i);
    full.text = text->str;
    full.selection = "selected text only";
    gchar *all_path = g_build_filename(argv[1], "all.pdf", NULL);
    if (export_pdf(&full, all_path, FALSE) < 3) return 5;
    PrintData selected = {0};
    selected.text = full.text;
    selected.selection = full.selection;
    gchar *selected_path = g_build_filename(argv[1], "selected.pdf", NULL);
    if (export_pdf(&selected, selected_path, TRUE) != 1) return 6;
    return 0;
}
