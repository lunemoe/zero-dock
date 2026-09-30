#include "zero-dock.h"
#include <X11/Xatom.h>
#include <glib/gstdio.h>
#include <sys/stat.h>

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
      data && n && format == 32)
    pid = (pid_t) * (unsigned long *)data;
  if (data)
    XFree(data);
  gdk_x11_display_error_trap_pop_ignored(gdk_display_get_default());
  return pid;
}
GDesktopAppInfo *zd_match_app(ZdDock *d, XfwWindow *w) {
  const gchar *const *ids = xfw_window_get_class_ids(w);
  GDesktopAppInfo *best = NULL;
  gint high = 0;
  for (GList *l = d->apps; l; l = l->next) {
    if (!G_IS_DESKTOP_APP_INFO(l->data))
      continue;
    GDesktopAppInfo *a = l->data;
    const gchar *wm = g_desktop_app_info_get_startup_wm_class(a),
                *id = g_app_info_get_id(G_APP_INFO(a)),
                *exe = g_app_info_get_executable(G_APP_INFO(a));
    gchar *base = exe ? g_path_get_basename(exe) : NULL;
    for (guint j = 0; ids && ids[j]; j++) {
      gint score = 0;
      if (wm && !g_ascii_strcasecmp(wm, ids[j]))
        score = 100;
      else if (base && !g_ascii_strcasecmp(base, ids[j]))
        score = 80;
      else if (id && zd_fuzzy_match(id, ids[j]))
        score = 40;
      if (score > high) {
        high = score;
        best = a;
      }
    }
    g_free(base);
  }
  return best ? g_object_ref(best) : NULL;
}
static void load_apps(ZdDock *d) {
  g_list_free_full(d->apps, g_object_unref);
  d->apps = g_app_info_get_all();
  for (GList *l = d->buttons; l; l = l->next) {
    ZdButton *b = l->data;
    if (b->window) {
      g_clear_object(&b->app);
      b->app = zd_match_app(d, b->window);
    }
  }
  zd_queue_refresh(d);
}
void zd_launch(ZdButton *b) {
  GDesktopAppInfo *a = b->app;
  if (!a) {
    g_warning("Zero Dock: no desktop entry matches this window");
    return;
  }
  GdkAppLaunchContext *ctx =
      gdk_display_get_app_launch_context(gdk_display_get_default());
  gdk_app_launch_context_set_timestamp(ctx, zd_timestamp(b->dock));
  GError *e = NULL;
  if (!g_app_info_launch(G_APP_INFO(a), NULL, G_APP_LAUNCH_CONTEXT(ctx), &e)) {
    g_warning("Zero Dock launch: %s", e->message);
    g_clear_error(&e);
  }
  g_object_unref(ctx);
}
void zd_pin_app(ZdDock *d, GDesktopAppInfo *a) {
  const gchar *path = g_desktop_app_info_get_filename(a);
  if (!path)
    return;
  for (GList *l = d->buttons; l; l = l->next) {
    ZdButton *b = l->data;
    if (b->pinned && !g_strcmp0(b->desktop, path))
      return;
  }
  if (zd_add_pin(d, path)) {
    zd_save(d);
    zd_update_buttons(d);
  }
}
void zd_unpin(ZdButton *b) {
  ZdDock *d = b->dock;
  if (!b->pinned)
    return;
  d->buttons = g_list_remove(d->buttons, b);
  zd_button_free(b);
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
void zd_save(ZdDock *d) {
  if (d->disposing || !d->rc_path)
    return;
  GKeyFile *f = g_key_file_new();
  zd_save_pins_to_keyfile(d, f);
  g_key_file_set_boolean(f, "Dock", "Previews", d->previews);
  g_key_file_set_boolean(f, "Dock", "Numbers", d->numbers);
  g_key_file_set_boolean(f, "Dock", "AllWorkspaces", d->all_workspaces);
  g_key_file_set_integer(f, "Dock", "PreviewWidth", d->preview_width);
  g_key_file_set_integer(f, "Dock", "Slots", d->slots);
  gsize len;
  gchar *s = g_key_file_to_data(f, &len, NULL);
  GError *e = NULL;
  if (!g_file_set_contents_full(d->rc_path, s, len,
                                G_FILE_SET_CONTENTS_CONSISTENT, 0600, &e)) {
    g_warning("Zero Dock settings: %s", e->message);
    g_clear_error(&e);
  }
  g_free(s);
  g_key_file_unref(f);
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
      g_hash_table_remove(d->windows, b->window);
      d->buttons = g_list_delete_link(d->buttons, l);
      zd_button_free(b);
    }
  }
  for (GList *l = live; l; l = l->next) {
    XfwWindow *w = l->data;
    XfwWindowType type = xfw_window_get_window_type(w);
    if (!xfw_window_is_skip_tasklist(w) && type != XFW_WINDOW_TYPE_DESKTOP &&
        type != XFW_WINDOW_TYPE_DOCK && !g_hash_table_contains(d->windows, w))
      zd_add_window(d, w);
  }
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
static void orientation_changed(XfcePanelPlugin *p, GtkOrientation o,
                                ZdDock *d) {
  (void)p;
  d->orientation = o;
  gtk_orientable_set_orientation(GTK_ORIENTABLE(d->box), o);
  zd_preview_hide(d);
  zd_update_buttons(d);
}
static void save_signal(XfcePanelPlugin *p, ZdDock *d) {
  (void)p;
  zd_save(d);
}
static void settings_changed(GtkToggleButton *b, ZdDock *d) {
  const gchar *k = g_object_get_data(G_OBJECT(b), "setting");
  gboolean v = gtk_toggle_button_get_active(b);
  if (!strcmp(k, "previews"))
    d->previews = v;
  else if (!strcmp(k, "numbers"))
    d->numbers = v;
  else
    d->all_workspaces = v;
  zd_preview_hide(d);
  zd_save(d);
  zd_update_buttons(d);
}
static void slots_changed(GtkSpinButton *spin, ZdDock *d) {
  d->slots = gtk_spin_button_get_value_as_int(spin);
  zd_save(d);
  zd_update_buttons(d);
}
static void settings_destroy(GtkWidget *w, ZdDock *d) {
  (void)w;
  d->settings = NULL;
}
void zd_configure(XfcePanelPlugin *p, ZdDock *d) {
  if (d->settings) {
    gtk_window_present(GTK_WINDOW(d->settings));
    return;
  }
  d->settings = gtk_dialog_new_with_buttons("Zero Dock 设置", NULL,
                                            GTK_DIALOG_DESTROY_WITH_PARENT,
                                            "关闭", GTK_RESPONSE_CLOSE, NULL);
  xfce_panel_plugin_take_window(p, GTK_WINDOW(d->settings));
  GtkWidget *box = gtk_dialog_get_content_area(GTK_DIALOG(d->settings));
  gtk_container_set_border_width(GTK_CONTAINER(box), 16);
  gtk_box_set_spacing(GTK_BOX(box), 12);
  const gchar *labels[] = {"显示可点击的窗口预览", "同应用多窗口显示序号",
                           "显示所有工作区的窗口"};
  const gchar *keys[] = {"previews", "numbers", "workspaces"};
  gboolean vals[] = {d->previews, d->numbers, d->all_workspaces};
  for (guint i = 0; i < 3; i++) {
    GtkWidget *b = gtk_check_button_new_with_label(labels[i]);
    gtk_toggle_button_set_active(GTK_TOGGLE_BUTTON(b), vals[i]);
    g_object_set_data(G_OBJECT(b), "setting", (gpointer)keys[i]);
    g_signal_connect(b, "toggled", G_CALLBACK(settings_changed), d);
    gtk_box_pack_start(GTK_BOX(box), b, FALSE, FALSE, 0);
  }
  GtkWidget *row = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8);
  gtk_box_pack_start(GTK_BOX(row),
                     gtk_label_new("预留图标位置（0 为自动宽度）"), TRUE, TRUE,
                     0);
  GtkWidget *spin = gtk_spin_button_new_with_range(0, 32, 1);
  gtk_spin_button_set_value(GTK_SPIN_BUTTON(spin), d->slots);
  g_signal_connect(spin, "value-changed", G_CALLBACK(slots_changed), d);
  gtk_box_pack_end(GTK_BOX(row), spin, FALSE, FALSE, 0);
  gtk_box_pack_start(GTK_BOX(box), row, FALSE, FALSE, 0);
  GtkWidget *info = gtk_label_new("每个窗口独立显示。拖入 .desktop "
                                  "文件可固定应用。\n喇叭滚轮调整应用音量，最高"
                                  " 200%。\n预览画面仅保存在内存中。");
  gtk_label_set_xalign(GTK_LABEL(info), 0);
  gtk_box_pack_start(GTK_BOX(box), info, FALSE, FALSE, 4);
  g_signal_connect_swapped(d->settings, "response",
                           G_CALLBACK(gtk_widget_destroy), d->settings);
  g_signal_connect(d->settings, "destroy", G_CALLBACK(settings_destroy), d);
  gtk_widget_show_all(d->settings);
}
static void about(XfcePanelPlugin *p, ZdDock *d) {
  (void)p;
  (void)d;
  gtk_show_about_dialog(NULL, "program-name", "Zero Dock 窗口停靠", "version",
                        "0.1.0", "comments",
                        "XFCE 原生面板插件 · C / GTK3 / "
                        "X11\n独立窗口、固定应用、预览与应用音量控制",
                        "license-type", GTK_LICENSE_MIT_X11, NULL);
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
  zd_popups_dispose(d);
  if (d->settings)
    gtk_widget_destroy(d->settings);
  if (d->menu)
    gtk_widget_destroy(d->menu);
  zd_audio_free(d->audio);
  d->audio = NULL;
  g_signal_handlers_disconnect_by_data(d->screen, d);
  g_signal_handlers_disconnect_by_data(d->app_monitor, d);
  g_list_free_full(d->buttons, (GDestroyNotify)zd_button_free);
  g_list_free_full(d->apps, g_object_unref);
  g_hash_table_destroy(d->windows);
  g_clear_object(&d->app_monitor);
  g_clear_object(&d->screen);
  g_clear_object(&d->css);
  g_free(d->rc_path);
  g_free(d);
}
void zd_construct(XfcePanelPlugin *p) {
  if (!GDK_IS_X11_DISPLAY(gdk_display_get_default())) {
    GtkWidget *w = gtk_label_new("Zero Dock 需要 X11");
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
  d->previews = d->numbers = d->all_workspaces = TRUE;
  d->preview_width = 300;
  d->slots = 10;
  d->windows = g_hash_table_new(g_direct_hash, g_direct_equal);
  d->apps = g_app_info_get_all();
  g_object_set_data(G_OBJECT(p), "zero-dock", d);
  const gchar *test_rc = g_object_get_data(G_OBJECT(p), "test-rc");
  d->rc_path =
      test_rc ? g_strdup(test_rc) : xfce_panel_plugin_save_location(p, TRUE);
  d->box = gtk_box_new(d->orientation, 2);
  gtk_widget_set_name(d->box, "zero-dock");
  gtk_widget_set_size_request(d->box, 12, 12);
  gtk_container_add(GTK_CONTAINER(p), d->box);
  d->css = gtk_css_provider_new();
  gtk_css_provider_load_from_data(
      d->css,
      "#zero-dock button {padding:0; margin:0; border:0; background:none; "
      "box-shadow:none;} #zero-dock button:hover "
      "{background:rgba(150,150,190,0.15); border-radius:8px;} #zero-dock "
      ".zd-sound {background:#303044;border-radius:8px; min-width:16px; "
      "min-height:16px;} .zd-popup {background:#292936; color:#eeeeee; "
      "border:1px solid #777088; border-radius:8px; padding:8px;} .zd-popup "
      "button {padding:4px;}",
      -1, NULL);
  gtk_style_context_add_provider(gtk_widget_get_style_context(d->box),
                                 GTK_STYLE_PROVIDER(d->css),
                                 GTK_STYLE_PROVIDER_PRIORITY_APPLICATION);
  GKeyFile *f = g_key_file_new();
  const gchar *initial = d->rc_path;
  const gchar *const *arguments = xfce_panel_plugin_get_arguments(p);
  if (!g_file_test(d->rc_path, G_FILE_TEST_EXISTS))
    for (guint i = 0; arguments && arguments[i]; i++)
      if (g_str_has_prefix(arguments[i], "--import-config="))
        initial = arguments[i] + strlen("--import-config=");
  if (initial && g_key_file_load_from_file(f, initial, G_KEY_FILE_NONE, NULL)) {
    if (g_key_file_has_key(f, "Dock", "Previews", NULL))
      d->previews = g_key_file_get_boolean(f, "Dock", "Previews", NULL);
    if (g_key_file_has_key(f, "Dock", "Numbers", NULL))
      d->numbers = g_key_file_get_boolean(f, "Dock", "Numbers", NULL);
    if (g_key_file_has_key(f, "Dock", "AllWorkspaces", NULL))
      d->all_workspaces =
          g_key_file_get_boolean(f, "Dock", "AllWorkspaces", NULL);
    if (g_key_file_has_key(f, "Dock", "PreviewWidth", NULL))
      d->preview_width = CLAMP(
          g_key_file_get_integer(f, "Dock", "PreviewWidth", NULL), 180, 600);
    if (g_key_file_has_key(f, "Dock", "Slots", NULL))
      d->slots = CLAMP(g_key_file_get_integer(f, "Dock", "Slots", NULL), 0, 32);
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
  const gchar *signals[] = {"window-opened",         "window-closed",
                            "active-window-changed", "window-stacking-changed",
                            "monitors-changed",      "notify::show-desktop"};
  for (guint i = 0; i < G_N_ELEMENTS(signals); i++)
    g_signal_connect_swapped(d->screen, signals[i],
                             G_CALLBACK(zd_queue_refresh), d);
  XfwWorkspaceManager *m = xfw_screen_get_workspace_manager(d->screen);
  for (GList *l = xfw_workspace_manager_list_workspace_groups(m); l;
       l = l->next)
    g_signal_connect_object(l->data, "active-workspace-changed",
                            G_CALLBACK(queue_from_plugin), p,
                            G_CONNECT_SWAPPED);
  d->app_monitor = g_app_info_monitor_get();
  g_signal_connect_swapped(d->app_monitor, "changed", G_CALLBACK(load_apps), d);
  d->audio = zd_audio_new(d);
  d->input = zd_input_new(d);
  d->active_frame_id = g_timeout_add_seconds(5, frame_timer, d);
  size_changed(p, xfce_panel_plugin_get_size(p), d);
  zd_refresh(d);
  gtk_widget_show(d->box);
  zd_save(d);
}
