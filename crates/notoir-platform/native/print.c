// GTK 3 owns the child process's main thread. Pango handles Unicode fallback,
// tabs, wrapping and line metrics; page breaks never bisect a line.
#include <gtk/gtk.h>
#include <math.h>
#include <string.h>
#include <stdlib.h>

typedef struct {
    gchar *text;
    gchar *selection;
    PangoLayout *layout;
    GArray *starts;
    GArray *ends;
} PrintData;

static gchar *read_text(const gchar *directory, const gchar *name, gboolean required) {
    gchar *path = g_build_filename(directory, name, NULL);
    gchar *text = NULL;
    GError *error = NULL;
    if (!g_file_get_contents(path, &text, NULL, &error) && required) {
        g_printerr("%s\n", error->message);
        exit(1);
    }
    g_clear_error(&error);
    g_free(path);
    return text;
}

static void begin_print(GtkPrintOperation *operation, GtkPrintContext *context, gpointer user_data) {
    PrintData *data = user_data;
    GtkPrintSettings *settings = gtk_print_operation_get_print_settings(operation);
    gboolean selected = gtk_print_settings_get_print_pages(settings) == GTK_PRINT_PAGES_SELECTION;
    const gchar *text = selected && data->selection ? data->selection : data->text;
    data->layout = gtk_print_context_create_pango_layout(context);
    PangoFontDescription *font = pango_font_description_from_string("Monospace 10");
    pango_layout_set_font_description(data->layout, font);
    pango_font_description_free(font);
    pango_layout_set_width(data->layout, (int)(gtk_print_context_get_width(context) * PANGO_SCALE));
    pango_layout_set_wrap(data->layout, PANGO_WRAP_WORD_CHAR);
    pango_layout_set_text(data->layout, text, -1);
    data->starts = g_array_new(FALSE, FALSE, sizeof(double));
    data->ends = g_array_new(FALSE, FALSE, sizeof(double));
    double start = 0;
    double end = 0;
    double height = gtk_print_context_get_height(context);
    PangoLayoutIter *iter = pango_layout_get_iter(data->layout);
    do {
        PangoRectangle logical;
        pango_layout_iter_get_line_extents(iter, NULL, &logical);
        double top = (double)logical.y / PANGO_SCALE;
        double bottom = (double)(logical.y + logical.height) / PANGO_SCALE;
        if (bottom - start > height && top > start) {
            g_array_append_val(data->starts, start);
            g_array_append_val(data->ends, top);
            start = top;
        }
        end = bottom;
    } while (pango_layout_iter_next_line(iter));
    pango_layout_iter_free(iter);
    g_array_append_val(data->starts, start);
    g_array_append_val(data->ends, end);
    gtk_print_operation_set_n_pages(operation, data->starts->len);
}

static void draw_page(GtkPrintOperation *operation, GtkPrintContext *context, gint page, gpointer user_data) {
    (void)operation;
    PrintData *data = user_data;
    cairo_t *cr = gtk_print_context_get_cairo_context(context);
    double start = g_array_index(data->starts, double, page);
    double end = g_array_index(data->ends, double, page);
    cairo_save(cr);
    cairo_rectangle(cr, 0, 0, gtk_print_context_get_width(context), end - start);
    cairo_clip(cr);
    cairo_set_source_rgb(cr, 0, 0, 0);
    cairo_move_to(cr, 0, -start);
    pango_cairo_show_layout(cr, data->layout);
    cairo_restore(cr);
}

__attribute__((constructor)) static void notoir_print_entry(int argc, char **argv) {
    if (argc != 3 || strcmp(argv[1], "--notoir-internal-print") != 0) return;
    if (!gtk_init_check(NULL, NULL)) {
        g_printerr("Could not connect to the desktop print dialog.\n");
        exit(1);
    }
    PrintData data = {0};
    data.text = read_text(argv[2], "text", TRUE);
    data.selection = read_text(argv[2], "selection", FALSE);
    gchar *title = read_text(argv[2], "title", TRUE);
    GtkPrintOperation *operation = gtk_print_operation_new();
    gtk_print_operation_set_job_name(operation, title);
    gtk_print_operation_set_unit(operation, GTK_UNIT_POINTS);
    gtk_print_operation_set_embed_page_setup(operation, TRUE);
    gtk_print_operation_set_support_selection(operation, TRUE);
    gtk_print_operation_set_has_selection(operation, data.selection != NULL);
    g_signal_connect(operation, "begin-print", G_CALLBACK(begin_print), &data);
    g_signal_connect(operation, "draw-page", G_CALLBACK(draw_page), &data);
    GError *error = NULL;
    GtkPrintOperationResult result = gtk_print_operation_run(operation,
        GTK_PRINT_OPERATION_ACTION_PRINT_DIALOG, NULL, &error);
    if (error) g_printerr("%s\n", error->message);
    exit(result == GTK_PRINT_OPERATION_RESULT_APPLY ? 0 :
        result == GTK_PRINT_OPERATION_RESULT_CANCEL ? 2 : 1);
}
