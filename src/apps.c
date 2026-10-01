#include "zero-dock.h"
#include <X11/Xatom.h>

gchar *zd_window_property(XfwWindow *w, const gchar *name) {
  GdkDisplay *gd = gdk_display_get_default();
  Display *x = GDK_DISPLAY_XDISPLAY(gd);
  Atom type;
  gint format;
  unsigned long count, rest;
  unsigned char *data = NULL;
  gchar *result = NULL;
  gdk_x11_display_error_trap_push(gd);
  if (XGetWindowProperty(x, xfw_window_x11_get_xid(w),
                         XInternAtom(x, name, False), 0, 1024, False,
                         AnyPropertyType, &type, &format, &count, &rest,
                         &data) == Success &&
      data && format == 8 && !rest && count > 0 &&
      (type == XA_STRING || type == XInternAtom(x, "UTF8_STRING", False)) &&
      g_utf8_validate((const gchar *)data, count, NULL))
    result = g_strndup((const gchar *)data, count);
  if (data)
    XFree(data);
  if (gdk_x11_display_error_trap_pop(gd))
    g_clear_pointer(&result, g_free);
  return result;
}
static gboolean exact_id(GDesktopAppInfo *app, const gchar *hint) {
  if (!hint || !*hint)
    return FALSE;
  const gchar *path = g_desktop_app_info_get_filename(app);
  const gchar *id = g_app_info_get_id(G_APP_INFO(app));
  gchar *base = path ? g_path_get_basename(path) : NULL;
  gchar *desktop = g_str_has_suffix(hint, ".desktop")
                       ? g_strdup(hint)
                       : g_strconcat(hint, ".desktop", NULL);
  gchar *flatpak = g_desktop_app_info_get_string(app, "X-Flatpak");
  gboolean match = !g_strcmp0(path, hint) || (id && !g_strcmp0(id, desktop)) ||
                   (base && !g_strcmp0(base, desktop)) ||
                   (flatpak && !g_strcmp0(flatpak, hint));
  g_free(base);
  g_free(desktop);
  g_free(flatpak);
  return match;
}
gboolean zd_app_equal(GDesktopAppInfo *a, GDesktopAppInfo *b) {
  if (!a || !b)
    return FALSE;
  const gchar *aa = g_desktop_app_info_get_filename(a),
              *bb = g_desktop_app_info_get_filename(b);
  return aa && bb ? !g_strcmp0(aa, bb)
                  : g_app_info_equal(G_APP_INFO(a), G_APP_INFO(b));
}
gint zd_app_match_score(GDesktopAppInfo *a, const gchar *const *ids) {
  const gchar *wm = g_desktop_app_info_get_startup_wm_class(a),
              *id = g_app_info_get_id(G_APP_INFO(a)),
              *exe = g_app_info_get_executable(G_APP_INFO(a));
  gchar *base = exe ? g_path_get_basename(exe) : NULL;
  gint high = 0;
  for (guint j = 0; ids && ids[j]; j++) {
    gint score = 0;
    if (wm && !g_ascii_strcasecmp(wm, ids[j]))
      score = 100;
    else if (base && !g_ascii_strcasecmp(base, ids[j]))
      score = 80;
    else if (id && zd_fuzzy_match(id, ids[j]))
      score = 40;
    high = MAX(high, score);
  }
  g_free(base);
  return high;
}
GDesktopAppInfo *zd_match_app(ZdDock *d, XfwWindow *w) {
  gchar *rule = zd_association_key(w);
  const gchar *path = rule && d->associations
                          ? g_hash_table_lookup(d->associations, rule)
                          : NULL;
  GDesktopAppInfo *manual =
      path ? g_desktop_app_info_new_from_filename(path) : NULL;
  g_free(rule);
  if (manual)
    return manual;
  const gchar *const *ids = xfw_window_get_class_ids(w);
  gchar *gtk_id = zd_window_property(w, "_GTK_APPLICATION_ID"),
        *desktop_id = zd_window_property(w, "_KDE_NET_WM_DESKTOP_FILE"),
        *startup = zd_window_property(w, "_NET_STARTUP_ID");
  pid_t pid = zd_window_pid(w);
  GDesktopAppInfo *best = NULL;
  gint high = 0;
  /* Include imported entries outside XDG application directories. A pin wins
   * ties, but a stronger installed StartupWMClass match still takes priority.
   */
  for (GList *l = d->buttons; l; l = l->next) {
    ZdButton *b = l->data;
    if (!b->pinned || !b->app)
      continue;
    gint score = zd_app_match_score(b->app, ids);
    if (exact_id(b->app, gtk_id) || exact_id(b->app, desktop_id))
      score = MAX(score, 160);
    if (b->launch_until > g_get_monotonic_time()) {
      if (startup && b->startup_id && !strcmp(startup, b->startup_id))
        score = MAX(score, 220);
      else if (b->launch_pid > 1 && zd_pid_descends(pid, b->launch_pid))
        score = MAX(score, 200);
    }
    if (score > high) {
      high = score;
      best = b->app;
    }
  }
  for (GList *l = d->apps; l; l = l->next) {
    if (!G_IS_DESKTOP_APP_INFO(l->data))
      continue;
    gint score = zd_app_match_score(l->data, ids);
    if (exact_id(l->data, gtk_id) || exact_id(l->data, desktop_id))
      score = MAX(score, 160);
    if (score > high) {
      high = score;
      best = l->data;
    }
  }
  g_free(gtk_id);
  g_free(desktop_id);
  g_free(startup);
  return best ? g_object_ref(best) : NULL;
}
void zd_reload_apps(ZdDock *d) {
  g_list_free_full(d->apps, g_object_unref);
  d->apps = g_app_info_get_all();
  d->app_generation++;
  for (GList *l = d->buttons; l; l = l->next) {
    ZdButton *b = l->data;
    if (b->pinned) {
      GDesktopAppInfo *app = g_desktop_app_info_new_from_filename(b->desktop);
      g_set_object(&b->app, app);
      g_clear_object(&app);
      b->icon_dirty = TRUE;
    }
  }
  zd_queue_refresh(d);
}
void zd_launch_complete(ZdButton *b) {
  b->launching = FALSE;
  g_clear_pointer(&b->launch_error, g_free);
  gtk_widget_queue_draw(b->drawing);
}
static gboolean launch_tick(gpointer data) {
  ZdDock *d = data;
  gboolean pending = FALSE;
  gint64 now = g_get_monotonic_time();
  for (GList *l = d->buttons; l; l = l->next) {
    ZdButton *b = l->data;
    if (!b->launching)
      continue;
    if (now >= b->launch_until) {
      b->launching = FALSE;
      g_free(b->launch_error);
      b->launch_error =
          g_strdup(_("启动请求已发送，但未检测到新窗口。可再次点击重试。"));
      zd_update_buttons(d);
    } else {
      pending = TRUE;
      gtk_widget_queue_draw(b->drawing);
    }
  }
  if (!pending) {
    d->launch_tick = 0;
    return G_SOURCE_REMOVE;
  }
  return G_SOURCE_CONTINUE;
}
static void launched(GAppLaunchContext *ctx, GAppInfo *app, GVariant *platform,
                     ZdButton *b) {
  (void)ctx;
  (void)app;
  const gchar *id = NULL;
  if (g_variant_lookup(platform, "startup-notification-id", "&s", &id)) {
    g_free(b->startup_id);
    b->startup_id = g_strdup(id);
  }
}
static void launch_pid(GDesktopAppInfo *app, GPid pid, gpointer data) {
  (void)app;
  ZdButton *b = data;
  b->launch_pid = pid;
}
void zd_launch(ZdButton *b) {
  ZdDock *d = b->dock;
  g_clear_pointer(&b->launch_error, g_free);
  /* Re-read idle pins so moved/deleted files fail visibly, rather than
   * launching stale commands held by the previous GDesktopAppInfo. */
  if (b->pinned) {
    GDesktopAppInfo *fresh = g_desktop_app_info_new_from_filename(b->desktop);
    g_set_object(&b->app, fresh);
    g_clear_object(&fresh);
    b->icon_dirty = TRUE;
  }
  if (!b->app) {
    b->launching = FALSE;
    b->launch_until = 0;
    b->launch_error =
        g_strdup(_("启动器文件已失效，请重新拖入该应用的 .desktop 文件。"));
    zd_show_error(d, b->launch_error);
    zd_update_buttons(d);
    return;
  }
  b->launching = TRUE;
  b->launch_until = g_get_monotonic_time() + d->launch_timeout * 1000;
  b->launch_pid = 0;
  g_clear_pointer(&b->startup_id, g_free);
  GdkAppLaunchContext *ctx =
      gdk_display_get_app_launch_context(gdk_display_get_default());
  gdk_app_launch_context_set_timestamp(ctx, zd_timestamp(d));
  g_signal_connect(ctx, "launched", G_CALLBACK(launched), b);
  GError *error = NULL;
  if (!g_desktop_app_info_launch_uris_as_manager(
          b->app, NULL, G_APP_LAUNCH_CONTEXT(ctx), G_SPAWN_SEARCH_PATH, NULL,
          NULL, launch_pid, b, &error)) {
    b->launching = FALSE;
    b->launch_until = 0;
    b->launch_error = g_strdup_printf(_("无法启动应用：%s"), error->message);
    zd_show_error(d, b->launch_error);
    g_clear_error(&error);
  } else if (!d->launch_tick)
    d->launch_tick = g_timeout_add(100, launch_tick, d);
  g_signal_handlers_disconnect_by_data(ctx, b);
  g_object_unref(ctx);
  zd_update_buttons(d);
}
