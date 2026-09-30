#include "zero-dock.h"
typedef enum {
  ACT_MIN,
  ACT_MAX,
  ACT_FULL,
  ACT_ABOVE,
  ACT_MUTE,
  ACT_LAUNCH,
  ACT_PIN,
  ACT_CLOSE,
  ACT_UNPIN,
  ACT_LEFT,
  ACT_RIGHT
} Action;
static void action(GtkMenuItem *item, ZdButton *b) {
  Action a = GPOINTER_TO_INT(g_object_get_data(G_OBJECT(item), "action"));
  if (b->closed)
    return;
  switch (a) {
  case ACT_MIN:
    if (xfw_window_is_minimized(b->window))
      zd_activate(b);
    else
      zd_minimize(b);
    break;
  case ACT_MAX:
    xfw_window_set_maximized(b->window, !xfw_window_is_maximized(b->window),
                             NULL);
    break;
  case ACT_FULL:
    xfw_window_set_fullscreen(b->window, !xfw_window_is_fullscreen(b->window),
                              NULL);
    break;
  case ACT_ABOVE:
    xfw_window_set_above(b->window, !xfw_window_is_above(b->window), NULL);
    break;
  case ACT_MUTE:
    zd_audio_mute(b);
    zd_volume_bubble(b);
    break;
  case ACT_LAUNCH:
    zd_launch(b);
    break;
  case ACT_PIN:
    if (b->app)
      zd_pin_app(b->dock, b->app);
    break;
  case ACT_CLOSE:
    xfw_window_close(b->window, zd_timestamp(b->dock), NULL);
    break;
  case ACT_UNPIN:
    zd_unpin(b);
    break;
  case ACT_LEFT:
  case ACT_RIGHT: {
    GList *l = g_list_find(b->dock->buttons, b);
    GList *other = a == ACT_LEFT ? l->prev : l->next;
    if (other)
      zd_move_button(b->dock, b, other->data, a == ACT_RIGHT);
    break;
  }
  }
}
static GtkWidget *add(GtkWidget *m, const gchar *label, Action a,
                      gboolean sensitive, ZdButton *b) {
  GtkWidget *i = gtk_menu_item_new_with_label(label);
  gtk_widget_set_sensitive(i, sensitive);
  g_object_set_data(G_OBJECT(i), "action", GINT_TO_POINTER(a));
  g_signal_connect(i, "activate", G_CALLBACK(action), b);
  gtk_menu_shell_append(GTK_MENU_SHELL(m), i);
  return i;
}
static void sep(GtkWidget *m) {
  gtk_menu_shell_append(GTK_MENU_SHELL(m), gtk_separator_menu_item_new());
}
static void menu_destroy(GtkWidget *w, ZdDock *d) {
  if (d->menu == w)
    d->menu = NULL;
}
static void done(GtkMenuShell *m, gpointer unused) {
  (void)unused;
  gtk_widget_destroy(GTK_WIDGET(m));
}
static GtkWidget *new_menu(ZdDock *d) {
  if (d->menu)
    gtk_widget_destroy(d->menu);
  GtkWidget *m = gtk_menu_new();
  d->menu = m;
  g_signal_connect(m, "destroy", G_CALLBACK(menu_destroy), d);
  g_signal_connect(m, "selection-done", G_CALLBACK(done), NULL);
  return m;
}
static void popup(ZdButton *b, GtkWidget *m, GdkEvent *e) {
  gtk_widget_show_all(m);
  xfce_panel_plugin_register_menu(b->dock->plugin, GTK_MENU(m));
  xfce_panel_plugin_popup_menu(b->dock->plugin, GTK_MENU(m), b->main, e);
}
static void move_ws(GtkMenuItem *i, ZdButton *b) {
  XfwWorkspace *w = g_object_get_data(G_OBJECT(i), "workspace");
  if (b->window && !b->closed)
    xfw_window_move_to_workspace(b->window, w, NULL);
}
void zd_window_menu(ZdButton *b, GdkEvent *e) {
  XfwWindow *w = b->window;
  if (!w || b->closed)
    return;
  GtkWidget *m = new_menu(b->dock);
  XfwWindowCapabilities c = xfw_window_get_capabilities(w);
  gboolean min = xfw_window_is_minimized(w), max = xfw_window_is_maximized(w),
           full = xfw_window_is_fullscreen(w), above = xfw_window_is_above(w);
  add(m, min ? "恢复窗口" : "最小化", ACT_MIN,
      c & (min ? XFW_WINDOW_CAPABILITIES_CAN_UNMINIMIZE
               : XFW_WINDOW_CAPABILITIES_CAN_MINIMIZE),
      b);
  add(m, max ? "还原大小" : "最大化", ACT_MAX,
      c & (max ? XFW_WINDOW_CAPABILITIES_CAN_UNMAXIMIZE
               : XFW_WINDOW_CAPABILITIES_CAN_MAXIMIZE),
      b);
  add(m, full ? "退出全屏" : "全屏", ACT_FULL,
      c & (full ? XFW_WINDOW_CAPABILITIES_CAN_UNFULLSCREEN
                : XFW_WINDOW_CAPABILITIES_CAN_FULLSCREEN),
      b);
  add(m, above ? "取消置顶" : "置顶", ACT_ABOVE,
      c & (above ? XFW_WINDOW_CAPABILITIES_CAN_UNPLACE_ABOVE
                 : XFW_WINDOW_CAPABILITIES_CAN_PLACE_ABOVE),
      b);
  GtkWidget *wi = gtk_menu_item_new_with_label("移到工作区"),
            *sub = gtk_menu_new();
  gtk_menu_item_set_submenu(GTK_MENU_ITEM(wi), sub);
  gtk_widget_set_sensitive(wi,
                           c & XFW_WINDOW_CAPABILITIES_CAN_CHANGE_WORKSPACE);
  gtk_menu_shell_append(GTK_MENU_SHELL(m), wi);
  XfwWorkspaceManager *wm = xfw_screen_get_workspace_manager(b->dock->screen);
  for (GList *l = xfw_workspace_manager_list_workspaces(wm); l; l = l->next) {
    XfwWorkspace *ws = l->data;
    const gchar *n = xfw_workspace_get_name(ws);
    gchar *fallback =
        g_strdup_printf("工作区 %u", xfw_workspace_get_number(ws) + 1);
    GtkWidget *i = gtk_check_menu_item_new_with_label(n && *n ? n : fallback);
    g_free(fallback);
    gtk_check_menu_item_set_active(GTK_CHECK_MENU_ITEM(i),
                                   ws == xfw_window_get_workspace(w));
    g_object_set_data_full(G_OBJECT(i), "workspace", g_object_ref(ws),
                           g_object_unref);
    g_signal_connect(i, "activate", G_CALLBACK(move_ws), b);
    gtk_menu_shell_append(GTK_MENU_SHELL(sub), i);
  }
  sep(m);
  b->audio = zd_audio_status(b);
  add(m, b->audio.muted ? "取消应用静音" : "应用静音", ACT_MUTE,
      b->audio.present, b);
  add(m, "启动新窗口", ACT_LAUNCH, b->app != NULL, b);
  add(m, "固定到启动器", ACT_PIN, b->app != NULL, b);
  sep(m);
  add(m, "关闭窗口", ACT_CLOSE, TRUE, b);
  popup(b, m, e);
}
static void desktop_action(GtkMenuItem *i, ZdButton *b) {
  const gchar *a = g_object_get_data(G_OBJECT(i), "desktop-action");
  GdkAppLaunchContext *c =
      gdk_display_get_app_launch_context(gdk_display_get_default());
  gdk_app_launch_context_set_timestamp(c, zd_timestamp(b->dock));
  g_desktop_app_info_launch_action(b->app, a, G_APP_LAUNCH_CONTEXT(c));
  g_object_unref(c);
}
void zd_pin_menu(ZdButton *b, GdkEvent *e) {
  GtkWidget *m = new_menu(b->dock);
  add(m, "启动新窗口", ACT_LAUNCH, TRUE, b);
  const gchar *const *actions = g_desktop_app_info_list_actions(b->app);
  for (guint j = 0; actions && actions[j]; j++) {
    gchar *name = g_desktop_app_info_get_action_name(b->app, actions[j]);
    GtkWidget *i = gtk_menu_item_new_with_label(name);
    g_free(name);
    g_object_set_data_full(G_OBJECT(i), "desktop-action", g_strdup(actions[j]),
                           g_free);
    g_signal_connect(i, "activate", G_CALLBACK(desktop_action), b);
    gtk_menu_shell_append(GTK_MENU_SHELL(m), i);
  }
  sep(m);
  GList *l = g_list_find(b->dock->buttons, b);
  add(m,
      b->dock->orientation == GTK_ORIENTATION_HORIZONTAL ? "向左移动"
                                                         : "向上移动",
      ACT_LEFT, l && l->prev, b);
  add(m,
      b->dock->orientation == GTK_ORIENTATION_HORIZONTAL ? "向右移动"
                                                         : "向下移动",
      ACT_RIGHT, l && l->next, b);
  sep(m);
  add(m, "取消固定", ACT_UNPIN, TRUE, b);
  popup(b, m, e);
}
void zd_minimize_all(ZdDock *d) {
  for (GList *l = d->buttons; l; l = l->next) {
    ZdButton *b = l->data;
    if (b->window && !xfw_window_is_minimized(b->window) &&
        (xfw_window_get_capabilities(b->window) &
         XFW_WINDOW_CAPABILITIES_CAN_MINIMIZE))
      zd_minimize(b);
  }
}
static void show_desktop(GtkMenuItem *i, ZdDock *d) {
  (void)i;
  zd_preview_hide(d);
  xfw_screen_set_show_desktop(d->screen,
                              !xfw_screen_get_show_desktop(d->screen));
}
void zd_menu_sync(ZdDock *d) {
  if (!d->show_desktop_item)
    return;
  g_signal_handlers_block_by_func(d->show_desktop_item,
                                  G_CALLBACK(show_desktop), d);
  gtk_check_menu_item_set_active(GTK_CHECK_MENU_ITEM(d->show_desktop_item),
                                 xfw_screen_get_show_desktop(d->screen));
  g_signal_handlers_unblock_by_func(d->show_desktop_item,
                                    G_CALLBACK(show_desktop), d);
}
static void min_all(GtkMenuItem *i, ZdDock *d) {
  (void)i;
  zd_minimize_all(d);
}
void zd_menu_install(ZdDock *d) {
  GtkWidget *i = gtk_check_menu_item_new_with_label("显示桌面");
  d->show_desktop_item = i;
  g_signal_connect(i, "activate", G_CALLBACK(show_desktop), d);
  xfce_panel_plugin_menu_insert_item(d->plugin, GTK_MENU_ITEM(i));
  gtk_widget_show(i);
  i = gtk_menu_item_new_with_label("最小化所有窗口");
  g_signal_connect(i, "activate", G_CALLBACK(min_all), d);
  xfce_panel_plugin_menu_insert_item(d->plugin, GTK_MENU_ITEM(i));
  gtk_widget_show(i);
}
