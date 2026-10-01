#include "zero-dock.h"
#include <X11/Xatom.h>
#include <glib/gstdio.h>
#include <sys/stat.h>

static void overflow_clicked(ZdDock *d) { zd_overflow_menu(d, NULL); }
static void monitors_changed(ZdDock *d) {
  zd_preview_hide(d);
  if (d->bubble)
    gtk_widget_hide(d->bubble);
  zd_queue_refresh(d);
}
ZdDock *zd_get_dock(XfcePanelPlugin *p) {
  return g_object_get_data(G_OBJECT(p), "zero-dock");
}
guint32 zd_timestamp(ZdDock *d) {
  (void)d;
  guint32 t = gtk_get_current_event_time();
  return t ? t : gdk_x11_get_server_time(gdk_get_default_root_window());
}
pid_t zd_window_pid(XfwWindow *w) {
  XfwApplication *a = xfw_window_get_application(w);
  XfwApplicationInstance *i = a ? xfw_application_get_instance(a, w) : NULL;
  if (i && xfw_application_instance_get_pid(i) > 0)
    return xfw_application_instance_get_pid(i);
  Display *x = GDK_DISPLAY_XDISPLAY(gdk_display_get_default());
  Atom type;
  gint format;
  unsigned long n, rest;
  unsigned char *data = NULL;
  pid_t pid = 0;
  gdk_x11_display_error_trap_push(gdk_display_get_default());
  if (XGetWindowProperty(
          x, xfw_window_x11_get_xid(w), XInternAtom(x, "_NET_WM_PID", False), 0,
          1, False, XA_CARDINAL, &type, &format, &n, &rest, &data) == Success &&
      data && n == 1 && format == 32 && type == XA_CARDINAL &&
      *(unsigned long *)data <= G_MAXINT)
    pid = (pid_t) * (unsigned long *)data;
  if (data)
    XFree(data);
  gdk_x11_display_error_trap_pop_ignored(gdk_display_get_default());
  return pid;
}
void zd_pin_app(ZdDock *d, GDesktopAppInfo *a) {
  const gchar *path = g_desktop_app_info_get_filename(a);
  if (!path)
    return;
  for (GList *l = d->buttons; l; l = l->next) {
    ZdButton *b = l->data;
    if (b->pinned && zd_app_equal(b->app, a))
      return;
  }
  /* Pin a running button in place, preserving its position and preview. */
  for (GList *l = d->buttons; l; l = l->next) {
    ZdButton *b = l->data;
    if (!b->window || b->pinned)
      continue;
    const gchar *const *ids = xfw_window_get_class_ids(b->window);
    gint score = zd_app_match_score(a, ids);
    if (zd_app_equal(b->app, a) ||
        (score > 0 && (!b->app || score >= zd_app_match_score(b->app, ids)))) {
      b->pinned = TRUE;
      g_set_object(&b->app, a);
      b->desktop = g_strdup(path);
      g_free(b->key);
      b->key = g_strconcat("pin:", path, NULL);
      d->app_generation++;
      b->icon_dirty = TRUE;
      zd_save(d);
      zd_refresh(d);
      return;
    }
  }
  if (zd_add_pin(d, path)) {
    zd_save(d);
    zd_refresh(d);
  }
}
void zd_unpin(ZdButton *b) {
  ZdDock *d = b->dock;
  if (!b->pinned)
    return;
  d->app_generation++;
  if (b->window) {
    b->pinned = FALSE;
    g_clear_pointer(&b->desktop, g_free);
    g_free(b->key);
    b->key = g_strdup_printf("window:%lu", xfw_window_x11_get_xid(b->window));
    if (d->menu)
      gtk_widget_destroy(d->menu);
  } else {
    d->buttons = g_list_remove(d->buttons, b);
    zd_button_free(b);
  }
  zd_save(d);
  zd_update_buttons(d);
}
void zd_move_button(ZdDock *d, ZdButton *s, ZdButton *t, gboolean after) {
  if (!s || !t || s == t || !g_list_find(d->buttons, s) ||
      !g_list_find(d->buttons, t))
    return;
  d->buttons = g_list_remove(d->buttons, s);
  gint p = g_list_index(d->buttons, t) + (after ? 1 : 0);
  d->buttons = g_list_insert(d->buttons, s, p);
  guint j = 0;
  for (GList *l = d->buttons; l; l = l->next)
    gtk_box_reorder_child(GTK_BOX(d->box), ((ZdButton *)l->data)->widget, j++);
  zd_save(d);
  zd_update_buttons(d);
}
void zd_save_pins_to_keyfile(ZdDock *d, GKeyFile *f) {
  GPtrArray *a = g_ptr_array_new();
  for (GList *l = d->buttons; l; l = l->next) {
    ZdButton *b = l->data;
    if (b->pinned)
      g_ptr_array_add(a, b->desktop);
  }
  g_key_file_set_string_list(f, "Dock", "Pinned",
                             (const gchar *const *)a->pdata, a->len);
  g_ptr_array_free(a, TRUE);
}
gboolean zd_save(ZdDock *d) {
  if (d->disposing || !d->rc_path || d->save_blocked)
    return FALSE;
  GKeyFile *f = g_key_file_new();
  g_key_file_load_from_file(f, d->rc_path, G_KEY_FILE_KEEP_COMMENTS, NULL);
  zd_save_pins_to_keyfile(d, f);
  g_key_file_set_boolean(f, "Dock", "Previews", d->previews);
  g_key_file_set_boolean(f, "Dock", "Numbers", d->numbers);
  g_key_file_set_boolean(f, "Dock", "AllWorkspaces", d->all_workspaces);
  g_key_file_set_integer(f, "Dock", "PreviewWidth", d->preview_width);
  g_key_file_set_integer(f, "Dock", "PreviewDelay", d->preview_delay);
  g_key_file_set_integer(f, "Dock", "PreviewInterval", d->preview_interval);
  g_key_file_set_integer(f, "Dock", "LaunchTimeout", d->launch_timeout);
  g_key_file_set_integer(f, "Dock", "Slots", d->slots);
  g_key_file_set_integer(f, "Dock", "MaxVisible", d->max_visible);
  g_key_file_set_integer(f, "Dock", "LeftAction", d->left_action);
  g_key_file_set_integer(f, "Dock", "MiddleAction", d->middle_action);
  g_key_file_set_boolean(f, "Dock", "ScrollWindows", d->scroll_windows);
  zd_associations_save(d, f);
  gsize len;
  gchar *s = g_key_file_to_data(f, &len, NULL);
  GError *e = NULL;
  gboolean saved = g_file_set_contents_full(
      d->rc_path, s, len, G_FILE_SET_CONTENTS_CONSISTENT, 0600, &e);
  if (!saved) {
    g_warning("Zero Dock settings: %s", e->message);
    g_clear_error(&e);
  }
  g_free(s);
  g_key_file_unref(f);
  return saved;
}
static void queue_from_plugin(XfcePanelPlugin *p) {
  ZdDock *d = zd_get_dock(p);
  if (d)
    zd_queue_refresh(d);
}
static gboolean frame_timer(gpointer data) {
  ZdDock *d = data;
  XfwWindow *w = xfw_screen_get_active_window(d->screen);
  ZdButton *b = w ? g_hash_table_lookup(d->windows, w) : NULL;
  if (b) {
    GdkPixbuf *p = zd_capture(b);
    g_clear_object(&p);
  }
  return G_SOURCE_CONTINUE;
}
void zd_refresh(ZdDock *d) {
  if (d->disposing)
    return;
  GList *live = xfw_screen_get_windows(d->screen), *next;
  for (GList *l = d->buttons; l; l = next) {
    next = l->next;
    ZdButton *b = l->data;
    if (b->window && (!g_list_find(live, b->window) ||
                      xfw_window_is_skip_tasklist(b->window))) {
      if (b->pinned)
        zd_button_release_window(b);
      else {
        d->buttons = g_list_delete_link(d->buttons, l);
        zd_button_free(b);
      }
    } else if (b->window && b->pinned &&
               (b->match_dirty || b->match_generation != d->app_generation)) {
      GDesktopAppInfo *app = zd_match_app(d, b->window);
      if (!zd_app_equal(b->app, app))
        zd_button_release_window(b);
      b->match_dirty = FALSE;
      b->match_generation = d->app_generation;
      g_clear_object(&app);
    } else if (b->window &&
               (b->match_dirty || b->match_generation != d->app_generation)) {
      GDesktopAppInfo *app = zd_match_app(d, b->window);
      /* Keep a manually imported app after unpinning while it still matches. */
      if (app || !b->app ||
          !zd_app_match_score(b->app, xfw_window_get_class_ids(b->window)))
        if (g_set_object(&b->app, app))
          b->icon_dirty = TRUE;
      b->match_dirty = FALSE;
      b->match_generation = d->app_generation;
      g_clear_object(&app);
    }
  }
  for (GList *l = live; l; l = l->next) {
    XfwWindow *w = l->data;
    XfwWindowType type = xfw_window_get_window_type(w);
    if (!xfw_window_is_skip_tasklist(w) && type != XFW_WINDOW_TYPE_DESKTOP &&
        type != XFW_WINDOW_TYPE_DOCK && !g_hash_table_contains(d->windows, w))
      zd_add_window(d, w);
  }
  /* When a pin's window closes, reuse it for another existing window of the
   * same app. Window references, signal handlers and cached frames move too. */
  for (GList *l = d->buttons; l; l = l->next) {
    ZdButton *pin = l->data;
    if (!pin->pinned || !pin->app ||
        (pin->window &&
         (d->all_workspaces || zd_window_in_workspace(pin->window))))
      continue;
    GList *candidate = NULL;
    for (GList *k = d->buttons; k; k = k->next) {
      ZdButton *b = k->data;
      if (b->pinned || !b->window || !zd_app_equal(pin->app, b->app))
        continue;
      if (!candidate)
        candidate = k;
      if (!d->all_workspaces && zd_window_in_workspace(b->window)) {
        candidate = k;
        break;
      }
    }
    if (!candidate)
      continue;
    ZdButton *b = candidate->data;
    if (pin->window && !zd_window_in_workspace(b->window))
      continue;
    XfwWindow *w = g_object_ref(b->window);
    GdkPixbuf *frame = b->thumbnail ? g_object_ref(b->thumbnail) : NULL;
    gint64 frame_time = b->thumbnail_time, old_time = pin->thumbnail_time;
    XfwWindow *old = pin->window ? g_object_ref(pin->window) : NULL;
    GdkPixbuf *old_frame = pin->thumbnail ? g_object_ref(pin->thumbnail) : NULL;
    zd_button_release_window(b);
    if (old) {
      zd_button_release_window(pin);
      zd_button_attach_window(b, old);
      b->thumbnail = old_frame;
      b->thumbnail_time = old_time;
      g_object_unref(old);
    } else {
      d->buttons = g_list_delete_link(d->buttons, candidate);
      zd_button_free(b);
      g_clear_object(&old_frame);
    }
    zd_button_attach_window(pin, w);
    pin->thumbnail = frame;
    pin->thumbnail_time = frame_time;
    g_object_unref(w);
  }
  guint position = 0;
  for (GList *l = d->buttons; l; l = l->next)
    gtk_box_reorder_child(GTK_BOX(d->box), ((ZdButton *)l->data)->widget,
                          position++);
  zd_update_buttons(d);
  zd_menu_sync(d);
}
static gboolean refresh_idle(gpointer p) {
  ZdDock *d = p;
  d->refresh_id = 0;
  zd_refresh(d);
  return G_SOURCE_REMOVE;
}
void zd_queue_refresh(ZdDock *d) {
  if (!d->disposing && !d->refresh_id)
    d->refresh_id = g_timeout_add(35, refresh_idle, d);
}
static gboolean size_changed(XfcePanelPlugin *p, gint size, ZdDock *d) {
  d->unit = MAX(28, size / MAX(1, xfce_panel_plugin_get_nrows(p)));
  gint requested = xfce_panel_plugin_get_icon_size(p);
  d->icon_size =
      MIN(d->unit - 8, requested > 0 ? MAX(16, requested) : d->unit - 8);
  zd_update_buttons(d);
  return TRUE;
}
static void icons_changed(XfcePanelPlugin *p, GParamSpec *pspec, ZdDock *d) {
  (void)pspec;
  size_changed(p, xfce_panel_plugin_get_size(p), d);
}
static void theme_changed(GtkIconTheme *theme, ZdDock *d) {
  (void)theme;
  for (GList *l = d->buttons; l; l = l->next)
    ((ZdButton *)l->data)->icon_dirty = TRUE;
  zd_update_buttons(d);
}
static void scale_changed(GtkWidget *widget, GParamSpec *pspec, ZdDock *d) {
  (void)widget;
  (void)pspec;
  theme_changed(d->icon_theme, d);
  for (GList *l = d->buttons; l; l = l->next)
    g_clear_object(&((ZdButton *)l->data)->thumbnail);
  zd_preview_hide(d);
}
static void orientation_changed(XfcePanelPlugin *p, GtkOrientation o,
                                ZdDock *d) {
  (void)p;
  d->orientation = o;
  gtk_orientable_set_orientation(GTK_ORIENTABLE(d->box), o);
  gtk_orientable_set_orientation(GTK_ORIENTABLE(d->container), o);
  zd_preview_hide(d);
  zd_update_buttons(d);
}
static void save_signal(XfcePanelPlugin *p, ZdDock *d) {
  (void)p;
  zd_save(d);
}
static void about_destroy(GtkWidget *widget, ZdDock *d) {
  (void)widget;
  d->about_dialog = NULL;
}
static void about(XfcePanelPlugin *p, ZdDock *d) {
  if (d->about_dialog) {
    gtk_window_present(GTK_WINDOW(d->about_dialog));
    return;
  }
  d->about_dialog = g_object_new(
      GTK_TYPE_ABOUT_DIALOG, "program-name", _("Zero Dock 窗口停靠"), "version",
      ZERO_DOCK_VERSION, "comments",
      _("XFCE 原生面板插件 · C / GTK3 / "
        "X11\n独立窗口、固定应用、预览与应用音量控制"),
      "license-type", GTK_LICENSE_MIT_X11, NULL);
  gtk_window_set_skip_taskbar_hint(GTK_WINDOW(d->about_dialog), TRUE);
  xfce_panel_plugin_take_window(p, GTK_WINDOW(d->about_dialog));
  g_signal_connect(d->about_dialog, "destroy", G_CALLBACK(about_destroy), d);
  g_signal_connect_swapped(d->about_dialog, "response",
                           G_CALLBACK(gtk_widget_destroy), d->about_dialog);
  gtk_widget_show(d->about_dialog);
}
static void dispose(XfcePanelPlugin *p, ZdDock *d) {
  (void)p;
  zd_save(d);
  d->disposing = TRUE;
  zd_input_free(d->input);
  d->input = NULL;
  g_object_set_data(G_OBJECT(d->plugin), "zero-dock", NULL);
  if (d->refresh_id)
    g_source_remove(d->refresh_id);
  if (d->active_frame_id)
    g_source_remove(d->active_frame_id);
  if (d->launch_tick)
    g_source_remove(d->launch_tick);
  zd_popups_dispose(d);
  if (d->association_dialog)
    gtk_widget_destroy(d->association_dialog);
  if (d->settings)
    gtk_widget_destroy(d->settings);
  if (d->error_dialog)
    gtk_widget_destroy(d->error_dialog);
  if (d->about_dialog)
    gtk_widget_destroy(d->about_dialog);
  if (d->menu)
    gtk_widget_destroy(d->menu);
  zd_audio_free(d->audio);
  d->audio = NULL;
  g_signal_handlers_disconnect_by_data(d->screen, d);
  g_signal_handlers_disconnect_by_data(d->app_monitor, d);
  g_signal_handlers_disconnect_by_data(d->icon_theme, d);
  g_list_free_full(d->buttons, (GDestroyNotify)zd_button_free);
  g_list_free_full(d->apps, g_object_unref);
  g_hash_table_destroy(d->windows);
  g_hash_table_destroy(d->associations);
  g_clear_object(&d->app_monitor);
  g_clear_object(&d->icon_theme);
  g_clear_object(&d->screen);
  g_clear_object(&d->css);
  g_free(d->rc_path);
  g_free(d);
}
void zd_construct(XfcePanelPlugin *p) {
  bindtextdomain(GETTEXT_PACKAGE, LOCALEDIR);
  bind_textdomain_codeset(GETTEXT_PACKAGE, "UTF-8");
  if (!GDK_IS_X11_DISPLAY(gdk_display_get_default())) {
    GtkWidget *w = gtk_label_new(_("Zero Dock 需要 X11"));
    gtk_container_add(GTK_CONTAINER(p), w);
    gtk_widget_show(w);
    return;
  }
  ZdDock *d = g_new0(ZdDock, 1);
  d->plugin = p;
  d->screen = xfw_screen_get_default();
  d->orientation = xfce_panel_plugin_get_orientation(p);
  d->unit = MAX(32, xfce_panel_plugin_get_size(p));
  d->icon_size = 32;
  zd_settings_defaults(d);
  d->app_generation = 1;
  d->windows = g_hash_table_new(g_direct_hash, g_direct_equal);
  d->associations =
      g_hash_table_new_full(g_str_hash, g_str_equal, g_free, g_free);
  const gchar *properties[] = {"_GTK_APPLICATION_ID",
                               "_KDE_NET_WM_DESKTOP_FILE", "_NET_STARTUP_ID",
                               "_NET_WM_PID"};
  for (guint i = 0; i < G_N_ELEMENTS(properties); i++)
    d->identity_atoms[i] = XInternAtom(
        GDK_DISPLAY_XDISPLAY(gdk_display_get_default()), properties[i], False);
  d->apps = g_app_info_get_all();
  d->icon_theme = g_object_ref(gtk_icon_theme_get_default());
  g_signal_connect(d->icon_theme, "changed", G_CALLBACK(theme_changed), d);
  g_object_set_data(G_OBJECT(p), "zero-dock", d);
  const gchar *test_rc = g_object_get_data(G_OBJECT(p), "test-rc");
  d->rc_path =
      test_rc ? g_strdup(test_rc) : xfce_panel_plugin_save_location(p, TRUE);
  d->box = gtk_box_new(d->orientation, 2);
  g_signal_connect(d->box, "notify::scale-factor", G_CALLBACK(scale_changed),
                   d);
  gtk_widget_set_name(d->box, "zero-dock");
  gtk_widget_set_size_request(d->box, 12, 12);
  d->container = gtk_box_new(d->orientation, 2);
  gtk_widget_set_name(d->container, "zero-dock");
  gtk_box_pack_start(GTK_BOX(d->container), d->box, FALSE, FALSE, 0);
  d->overflow = gtk_button_new_with_label("…");
  gtk_widget_set_no_show_all(d->overflow, TRUE);
  g_signal_connect(d->overflow, "key-press-event", G_CALLBACK(zd_focus_key), d);
  gtk_box_pack_end(GTK_BOX(d->container), d->overflow, FALSE, FALSE, 0);
  g_signal_connect_swapped(d->overflow, "clicked", G_CALLBACK(overflow_clicked),
                           d);
  gtk_container_add(GTK_CONTAINER(p), d->container);
  d->css = gtk_css_provider_new();
  gtk_css_provider_load_from_data(
      d->css,
      "#zero-dock button {padding:0; margin:0; border:0; background:none; "
      "box-shadow:none;} #zero-dock button:hover "
      "{background:alpha(@theme_selected_bg_color,0.18); border-radius:8px;} "
      "#zero-dock "
      ".zd-sound "
      "{background:@theme_bg_color;color:@theme_fg_color;border-radius:8px; "
      "min-width:16px; "
      "min-height:16px;} .zd-popup {background:@theme_bg_color; "
      "color:@theme_fg_color; "
      "border:1px solid shade(@theme_bg_color,0.65); border-radius:8px; "
      "padding:8px;} #zero-dock button:focus {box-shadow:inset 0 0 0 2px "
      "@theme_selected_bg_color;} .zd-popup "
      "button {padding:4px;}",
      -1, NULL);
  gtk_style_context_add_provider(gtk_widget_get_style_context(d->box),
                                 GTK_STYLE_PROVIDER(d->css),
                                 GTK_STYLE_PROVIDER_PRIORITY_APPLICATION);
  gtk_style_context_add_provider(gtk_widget_get_style_context(d->overflow),
                                 GTK_STYLE_PROVIDER(d->css),
                                 GTK_STYLE_PROVIDER_PRIORITY_APPLICATION);
  GKeyFile *f = g_key_file_new();
  const gchar *initial = d->rc_path;
  const gchar *const *arguments = xfce_panel_plugin_get_arguments(p);
  if (!g_file_test(d->rc_path, G_FILE_TEST_EXISTS))
    for (guint i = 0; arguments && arguments[i]; i++)
      if (g_str_has_prefix(arguments[i], "--import-config="))
        initial = arguments[i] + strlen("--import-config=");
  if (initial && zd_settings_read(d, initial, f)) {
    zd_settings_load(d, f);
    zd_associations_load(d, f);
    gsize n;
    gchar **pins = g_key_file_get_string_list(f, "Dock", "Pinned", &n, NULL);
    for (gsize i = 0; pins && i < n; i++)
      zd_add_pin(d, pins[i]);
    g_strfreev(pins);
  }
  g_key_file_unref(f);
  xfce_panel_plugin_set_expand(p, FALSE);
  xfce_panel_plugin_set_shrink(p, TRUE);
  xfce_panel_plugin_menu_show_configure(p);
  xfce_panel_plugin_menu_show_about(p);
  xfce_panel_plugin_add_action_widget(p, d->box);
  xfce_panel_plugin_add_action_widget(p, d->overflow);
  zd_menu_install(d);
  g_signal_connect(p, "free-data", G_CALLBACK(dispose), d);
  g_signal_connect(p, "save", G_CALLBACK(save_signal), d);
  g_signal_connect(p, "configure-plugin", G_CALLBACK(zd_configure), d);
  g_signal_connect(p, "about", G_CALLBACK(about), d);
  g_signal_connect(p, "size-changed", G_CALLBACK(size_changed), d);
  g_signal_connect(p, "notify::icon-size", G_CALLBACK(icons_changed), d);
  g_signal_connect(p, "notify::nrows", G_CALLBACK(icons_changed), d);
  g_signal_connect(p, "orientation-changed", G_CALLBACK(orientation_changed),
                   d);
  const gchar *signals[] = {"window-opened", "window-closed",
                            "active-window-changed", "window-stacking-changed",
                            "notify::show-desktop"};
  for (guint i = 0; i < G_N_ELEMENTS(signals); i++)
    g_signal_connect_swapped(d->screen, signals[i],
                             G_CALLBACK(zd_queue_refresh), d);
  g_signal_connect_swapped(d->screen, "monitors-changed",
                           G_CALLBACK(monitors_changed), d);
  XfwWorkspaceManager *m = xfw_screen_get_workspace_manager(d->screen);
  for (GList *l = xfw_workspace_manager_list_workspace_groups(m); l;
       l = l->next)
    g_signal_connect_object(l->data, "active-workspace-changed",
                            G_CALLBACK(queue_from_plugin), p,
                            G_CONNECT_SWAPPED);
  d->app_monitor = g_app_info_monitor_get();
  g_signal_connect_swapped(d->app_monitor, "changed",
                           G_CALLBACK(zd_reload_apps), d);
  d->audio = zd_audio_new(d);
  d->input = zd_input_new(d);
  d->active_frame_id = g_timeout_add_seconds(5, frame_timer, d);
  size_changed(p, xfce_panel_plugin_get_size(p), d);
  zd_refresh(d);
  gtk_widget_show(d->box);
  gtk_widget_show(d->container);
  zd_save(d);
}
