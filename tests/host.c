#include "../src/zero-dock.h"
#include <glib/gstdio.h>
#include <libxfce4panel/xfce-panel-plugin-provider.h>
static gboolean finish(gpointer data) {
  gtk_widget_destroy(data);
  return G_SOURCE_REMOVE;
}
static void destroy(GtkWidget *w, gpointer p) {
  (void)w;
  (void)p;
  gtk_main_quit();
}
int main(int argc, char **argv) {
  gtk_init(&argc, &argv);
  gchar *temporary = NULL, *temporary_rc = NULL;
  if (argc < 2) {
    temporary = g_dir_make_tmp("zero-dock-host-XXXXXX", NULL);
    g_assert_nonnull(temporary);
    temporary_rc = g_build_filename(temporary, "host.rc", NULL);
  }
  GtkWidget *w = gtk_window_new(GTK_WINDOW_TOPLEVEL);
  gtk_window_set_title(GTK_WINDOW(w), "Zero Dock 独立测试台");
  gtk_window_set_default_size(GTK_WINDOW(w), 800, 64);
  gtk_window_set_type_hint(GTK_WINDOW(w), GDK_WINDOW_TYPE_HINT_UTILITY);
  XfcePanelPlugin *p =
      g_object_new(XFCE_TYPE_PANEL_PLUGIN, "name", "zero-dock", "unique-id",
                   9001, "display-name", "Zero Dock 测试", "comment",
                   "Independent test host", NULL);
  g_object_set_data(G_OBJECT(p), "test-rc", argc > 1 ? argv[1] : temporary_rc);
  gtk_container_add(GTK_CONTAINER(w), GTK_WIDGET(p));
  xfce_panel_plugin_provider_set_size(XFCE_PANEL_PLUGIN_PROVIDER(p), 48);
  zd_construct(p);
  g_signal_connect(w, "destroy", G_CALLBACK(destroy), NULL);
  gtk_widget_show_all(w);
  if (argc > 2)
    g_timeout_add_seconds(MAX(1, atoi(argv[2])), finish, w);
  gtk_main();
  if (temporary) {
    g_remove(temporary_rc);
    g_rmdir(temporary);
    g_free(temporary_rc);
    g_free(temporary);
  }
  return 0;
}
