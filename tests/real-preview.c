#include "../src/zero-dock.h"
#include <glib/gstdio.h>
#include <libxfce4panel/xfce-panel-plugin-provider.h>
static gboolean blue;
static gboolean paint(GtkWidget *w, cairo_t *cr, gpointer user) {
  (void)w;
  (void)user;
  cairo_set_source_rgb(cr, blue ? .08 : .9, .12, blue ? .9 : .08);
  cairo_paint(cr);
  return FALSE;
}
static void spin(guint ms) {
  gint64 until = g_get_monotonic_time() + ms * 1000;
  do {
    while (g_main_context_iteration(NULL, FALSE))
      ;
    g_usleep(2000);
  } while (g_get_monotonic_time() < until);
}
int main(int argc, char **argv) {
  gtk_init(&argc, &argv);
  gchar *temporary = g_dir_make_tmp("zero-dock-preview-XXXXXX", NULL);
  g_assert_nonnull(temporary);
  gchar *rc = g_build_filename(temporary, "preview.rc", NULL);
  GtkWidget *w = gtk_window_new(GTK_WINDOW_TOPLEVEL),
            *area = gtk_drawing_area_new();
  gtk_window_set_title(GTK_WINDOW(w), "Zero Dock 实时预览验证");
  gtk_window_set_default_size(GTK_WINDOW(w), 320, 220);
  gtk_container_add(GTK_CONTAINER(w), area);
  g_signal_connect(area, "draw", G_CALLBACK(paint), NULL);
  gtk_widget_show_all(w);
  spin(350);
  GtkWidget *host = gtk_window_new(GTK_WINDOW_TOPLEVEL);
  gtk_window_set_skip_taskbar_hint(GTK_WINDOW(host), TRUE);
  gtk_window_move(GTK_WINDOW(host), 10, 10);
  XfcePanelPlugin *p =
      g_object_new(XFCE_TYPE_PANEL_PLUGIN, "name", "zero-dock", "unique-id",
                   9199, "display-name", "test", "comment", "test", NULL);
  g_object_set_data(G_OBJECT(p), "test-rc", rc);
  gtk_container_add(GTK_CONTAINER(host), GTK_WIDGET(p));
  xfce_panel_plugin_provider_set_size(XFCE_PANEL_PLUGIN_PROVIDER(p), 48);
  zd_construct(p);
  gtk_widget_show_all(host);
  spin(350);
  ZdDock *d = zd_get_dock(p);
  ZdButton *b = NULL;
  Window xid = GDK_WINDOW_XID(gtk_widget_get_window(w));
  for (GList *l = d->buttons; l; l = l->next) {
    ZdButton *q = l->data;
    if (q->window && xfw_window_x11_get_xid(q->window) == xid)
      b = q;
  }
  g_assert_nonnull(b);
  GdkPixbuf *red = zd_capture(b);
  g_assert_nonnull(red);
  guchar *r = gdk_pixbuf_get_pixels(red) +
              gdk_pixbuf_get_rowstride(red) * (gdk_pixbuf_get_height(red) / 2) +
              gdk_pixbuf_get_n_channels(red) * (gdk_pixbuf_get_width(red) / 2);
  g_assert_cmpint(r[0], >, r[2]);
  blue = TRUE;
  gtk_widget_queue_draw(area);
  spin(300);
  GdkPixbuf *frame = zd_capture(b);
  g_assert_nonnull(frame);
  guchar *v =
      gdk_pixbuf_get_pixels(frame) +
      gdk_pixbuf_get_rowstride(frame) * (gdk_pixbuf_get_height(frame) / 2) +
      gdk_pixbuf_get_n_channels(frame) * (gdk_pixbuf_get_width(frame) / 2);
  g_assert_cmpint(v[2], >, v[0]);
  g_assert_cmpint(r[0], !=, v[0]);
  if (argc > 1)
    gdk_pixbuf_save(frame, argv[1], "png", NULL, NULL);
  g_print("PASS real desktop compositor: red -> blue frame pixels changed; "
          "thumbnail %dx%d\n",
          gdk_pixbuf_get_width(frame), gdk_pixbuf_get_height(frame));
  g_object_unref(red);
  g_object_unref(frame);
  gtk_widget_destroy(host);
  gtk_widget_destroy(w);
  spin(100);
  g_remove(rc);
  g_rmdir(temporary);
  g_free(rc);
  g_free(temporary);
  return 0;
}
