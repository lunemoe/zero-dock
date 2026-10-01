#include "zero-dock.h"

gchar *zd_association_key(XfwWindow *window) {
  const gchar *const *ids = xfw_window_get_class_ids(window);
  GString *text = g_string_new(NULL);
  for (guint i = 0; ids && ids[i]; i++) {
    gchar *lower = g_utf8_strdown(ids[i], -1);
    g_string_append_printf(text, "%zu:%s;", strlen(lower), lower);
    g_free(lower);
  }
  gchar *key = text->len ? g_compute_checksum_for_string(G_CHECKSUM_SHA256,
                                                         text->str, text->len)
                         : NULL;
  g_string_free(text, TRUE);
  return key;
}
static gboolean valid_key(const gchar *key) {
  if (strlen(key) != 64)
    return FALSE;
  for (guint i = 0; key[i]; i++)
    if (!g_ascii_isxdigit(key[i]))
      return FALSE;
  return TRUE;
}
void zd_associations_load(ZdDock *d, GKeyFile *file) {
  g_hash_table_remove_all(d->associations);
  gsize n = 0;
  gchar **keys = g_key_file_get_keys(file, "Associations", &n, NULL);
  for (gsize i = 0; keys && i < MIN(n, 512); i++) {
    gchar *path = g_key_file_get_string(file, "Associations", keys[i], NULL);
    if (valid_key(keys[i]) && path && g_path_is_absolute(path) &&
        g_str_has_suffix(path, ".desktop"))
      g_hash_table_insert(d->associations, g_strdup(keys[i]), path);
    else
      g_free(path);
  }
  g_strfreev(keys);
}
void zd_associations_save(ZdDock *d, GKeyFile *file) {
  g_key_file_remove_group(file, "Associations", NULL);
  if (!d->associations)
    return;
  GHashTableIter iter;
  gpointer key, value;
  g_hash_table_iter_init(&iter, d->associations);
  while (g_hash_table_iter_next(&iter, &key, &value))
    g_key_file_set_string(file, "Associations", key, value);
}
gboolean zd_associate(ZdDock *d, XfwWindow *window, const gchar *path,
                      GError **error) {
  gchar *key = zd_association_key(window);
  GDesktopAppInfo *app = path && g_path_is_absolute(path)
                             ? g_desktop_app_info_new_from_filename(path)
                             : NULL;
  if (!key || (path && (!g_str_has_suffix(path, ".desktop") || !app))) {
    g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                        _("窗口缺少可保存的窗口类，或所选桌面文件无效。"));
    g_free(key);
    g_clear_object(&app);
    return FALSE;
  }
  g_clear_object(&app);
  if (path)
    g_hash_table_replace(d->associations, key, g_strdup(path));
  else {
    g_hash_table_remove(d->associations, key);
    g_free(key);
  }
  d->app_generation++;
  zd_save(d);
  zd_queue_refresh(d);
  return TRUE;
}
void zd_association_clear(ZdButton *b) {
  if (b->window)
    zd_associate(b->dock, b->window, NULL, NULL);
}
static void destroyed(GtkWidget *widget, ZdDock *d) {
  (void)widget;
  d->association_dialog = NULL;
}
static void response(GtkDialog *dialog, gint result, ZdDock *d) {
  if (result == GTK_RESPONSE_ACCEPT) {
    XfwWindow *window = g_object_get_data(G_OBJECT(dialog), "window");
    gchar *path = gtk_file_chooser_get_filename(GTK_FILE_CHOOSER(dialog));
    GError *error = NULL;
    if (!g_hash_table_contains(d->windows, window))
      zd_show_error(d, _("窗口已关闭，请为仍在运行的窗口设置关联。"));
    else if (!zd_associate(d, window, path, &error)) {
      zd_show_error(d, error->message);
      g_clear_error(&error);
    }
    g_free(path);
  }
  gtk_widget_destroy(GTK_WIDGET(dialog));
}
void zd_association_choose(ZdButton *b) {
  ZdDock *d = b->dock;
  if (!b->window)
    return;
  if (d->association_dialog)
    gtk_widget_destroy(d->association_dialog);
  d->association_dialog = gtk_file_chooser_dialog_new(
      _("选择关联应用的桌面文件"), NULL, GTK_FILE_CHOOSER_ACTION_OPEN,
      _("取消"), GTK_RESPONSE_CANCEL, _("关联"), GTK_RESPONSE_ACCEPT, NULL);
  GtkFileFilter *filter = gtk_file_filter_new();
  gtk_file_filter_set_name(filter, _("应用启动器 (*.desktop)"));
  gtk_file_filter_add_pattern(filter, "*.desktop");
  gtk_file_chooser_add_filter(GTK_FILE_CHOOSER(d->association_dialog), filter);
  gtk_file_chooser_set_current_folder(GTK_FILE_CHOOSER(d->association_dialog),
                                      "/usr/share/applications");
  GtkWidget *scope =
      gtk_label_new(_("此规则适用于相同窗口类的所有窗口。\n窗口类完全相同的多配"
                      "置应用仍需要应用自身提供不同身份。"));
  gtk_file_chooser_set_extra_widget(GTK_FILE_CHOOSER(d->association_dialog),
                                    scope);
  gtk_widget_show(scope);
  gtk_window_set_skip_taskbar_hint(GTK_WINDOW(d->association_dialog), TRUE);
  xfce_panel_plugin_take_window(d->plugin, GTK_WINDOW(d->association_dialog));
  g_object_set_data_full(G_OBJECT(d->association_dialog), "window",
                         g_object_ref(b->window), g_object_unref);
  g_signal_connect(d->association_dialog, "response", G_CALLBACK(response), d);
  g_signal_connect(d->association_dialog, "destroy", G_CALLBACK(destroyed), d);
  gtk_widget_show(d->association_dialog);
}
