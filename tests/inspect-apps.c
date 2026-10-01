/* Read-only inspection of existing windows. No titles, captures or window
 * actions. */
#include "../src/zero-dock.h"
int main(int argc, char **argv) {
  gtk_init(&argc, &argv);
  if (!GDK_IS_X11_DISPLAY(gdk_display_get_default()))
    return 77;
  ZdDock d = {0};
  d.screen = xfw_screen_get_default();
  d.apps = g_app_info_get_all();
  d.associations =
      g_hash_table_new_full(g_str_hash, g_str_equal, g_free, g_free);
  gint64 deadline = g_get_monotonic_time() + 500000;
  while (g_get_monotonic_time() < deadline) {
    while (g_main_context_iteration(NULL, FALSE))
      ;
    g_usleep(5000);
  }
  const gchar *labels[] = {"Chrome", "Thunar", "Steam", "Codex", "Other"};
  guint counts[5] = {0}, matched[5] = {0};
  for (GList *l = xfw_screen_get_windows(d.screen); l; l = l->next) {
    XfwWindow *window = l->data;
    if (xfw_window_is_skip_tasklist(window))
      continue;
    guint group = 4;
    const gchar *const *ids = xfw_window_get_class_ids(window);
    for (guint i = 0; ids && ids[i]; i++) {
      gchar *id = g_ascii_strdown(ids[i], -1);
      if (strstr(id, "chrome"))
        group = 0;
      else if (strstr(id, "thunar"))
        group = 1;
      else if (strstr(id, "steam"))
        group = 2;
      else if (strstr(id, "codex"))
        group = 3;
      g_free(id);
    }
    GDesktopAppInfo *app = zd_match_app(&d, window);
    counts[group]++;
    matched[group] += app != NULL;
    g_clear_object(&app);
  }
  for (guint i = 0; i < 5; i++)
    g_print("%s: %u windows, %u associated with a desktop entry\n", labels[i],
            counts[i], matched[i]);
  g_list_free_full(d.apps, g_object_unref);
  g_hash_table_unref(d.associations);
  g_object_unref(d.screen);
  return 0;
}
