#include "zero-dock.h"

void zd_popup_menu(ZdDock *d, GtkWidget *menu, GtkWidget *anchor,
                   GdkEvent *event) {
  GdkEvent *owned = NULL;
  if (!event) {
    owned = gtk_get_current_event();
    if (!owned) {
      owned = gdk_event_new(GDK_BUTTON_PRESS);
      owned->button.window = g_object_ref(gtk_widget_get_window(anchor));
      owned->button.button = 0;
      owned->button.time = zd_timestamp(d);
      gdk_event_set_device(
          owned, gdk_seat_get_pointer(
                     gdk_display_get_default_seat(gdk_display_get_default())));
    }
    event = owned;
  }
  gtk_widget_show_all(menu);
  xfce_panel_plugin_register_menu(d->plugin, GTK_MENU(menu));
  xfce_panel_plugin_popup_menu(d->plugin, GTK_MENU(menu), anchor, event);
  if (owned)
    gdk_event_free(owned);
}
void zd_layout_update(ZdDock *d) {
  guint eligible = 0, shown = 0;
  for (GList *l = d->buttons; l; l = l->next) {
    ZdButton *b = l->data;
    b->eligible = !b->window || b->pinned || d->all_workspaces ||
                  zd_window_in_workspace(b->window);
    eligible += b->eligible;
  }
  gboolean overflow = d->max_visible && eligible > d->max_visible;
  guint limit = overflow ? d->max_visible - 1 : eligible;
  for (GList *l = d->buttons; l; l = l->next) {
    ZdButton *b = l->data;
    gboolean visible = b->eligible && shown < limit;
    if (visible)
      shown++;
    if (!visible && d->hover_button == b)
      zd_preview_hide(d);
    gtk_widget_set_visible(b->widget, visible);
  }
  gtk_widget_set_visible(d->overflow, overflow);
  gtk_widget_set_size_request(d->overflow, d->unit, d->unit);
  gchar *tip = g_strdup_printf(_("更多窗口与启动器（%u）"), eligible - shown);
  gtk_widget_set_tooltip_text(d->overflow, tip);
  atk_object_set_name(gtk_widget_get_accessible(d->overflow), tip);
  g_free(tip);
  guint slots = MAX(d->slots, shown + overflow);
  if (d->max_visible)
    slots = MIN(slots, d->max_visible);
  gint length = MAX(12, slots * (d->unit + 2));
  gtk_widget_set_size_request(d->box, 12, 12);
  gtk_widget_set_size_request(
      d->container, d->orientation == GTK_ORIENTATION_HORIZONTAL ? length : 12,
      d->orientation == GTK_ORIENTATION_VERTICAL ? length : 12);
}
static void menu_destroy(GtkWidget *widget, ZdDock *d) {
  if (d->menu == widget)
    d->menu = NULL;
}
static void selection_done(GtkWidget *widget, gpointer data) {
  (void)data;
  gtk_widget_destroy(widget);
}
static void select_item(GtkMenuItem *item, ZdDock *d) {
  const gchar *key = g_object_get_data(G_OBJECT(item), "button-key");
  for (GList *l = d->buttons; l; l = l->next) {
    ZdButton *b = l->data;
    if (!g_strcmp0(key, b->key)) {
      if (b->window)
        zd_activate(b);
      else
        zd_toggle(b);
      return;
    }
  }
}
void zd_overflow_menu(ZdDock *d, GdkEvent *event) {
  zd_preview_hide(d);
  if (d->menu)
    gtk_widget_destroy(d->menu);
  d->menu = gtk_menu_new();
  g_signal_connect(d->menu, "destroy", G_CALLBACK(menu_destroy), d);
  g_signal_connect(d->menu, "selection-done", G_CALLBACK(selection_done), NULL);
  for (GList *l = d->buttons; l; l = l->next) {
    ZdButton *b = l->data;
    if (!b->eligible || gtk_widget_get_visible(b->widget))
      continue;
    const gchar *name = b->window ? xfw_window_get_name(b->window)
                        : b->app
                            ? g_app_info_get_display_name(G_APP_INFO(b->app))
                            : _("启动器文件已失效");
    XfwWorkspace *ws = b->window ? xfw_window_get_workspace(b->window) : NULL;
    const gchar *workspace = ws ? xfw_workspace_get_name(ws) : NULL;
    gchar *label = workspace ? g_strdup_printf("[%s] %s", workspace,
                                               name ? name : _("无标题窗口"))
                             : g_strdup(name ? name : _("无标题窗口"));
    GtkWidget *item = gtk_check_menu_item_new_with_label(label);
    g_free(label);
    gtk_check_menu_item_set_active(GTK_CHECK_MENU_ITEM(item),
                                   b->window &&
                                       xfw_window_is_active(b->window));
    g_object_set_data_full(G_OBJECT(item), "button-key", g_strdup(b->key),
                           g_free);
    g_signal_connect(item, "activate", G_CALLBACK(select_item), d);
    gtk_menu_shell_append(GTK_MENU_SHELL(d->menu), item);
  }
  zd_popup_menu(d, d->menu, d->overflow, event);
}

gboolean zd_focus_key(GtkWidget *widget, GdkEventKey *event, ZdDock *d) {
  gint delta =
      (event->keyval == GDK_KEY_Right || event->keyval == GDK_KEY_Down) ? 1
      : (event->keyval == GDK_KEY_Left || event->keyval == GDK_KEY_Up)  ? -1
                                                                        : 0;
  if (!delta && event->keyval != GDK_KEY_Home && event->keyval != GDK_KEY_End)
    return FALSE;
  GPtrArray *items = g_ptr_array_new();
  for (GList *l = d->buttons; l; l = l->next) {
    ZdButton *q = l->data;
    if (gtk_widget_get_visible(q->widget))
      g_ptr_array_add(items, q->main);
  }
  if (gtk_widget_get_visible(d->overflow))
    g_ptr_array_add(items, d->overflow);
  for (guint i = 0; i < items->len; i++)
    if (g_ptr_array_index(items, i) == widget) {
      gint next = event->keyval == GDK_KEY_Home ? 0
                  : event->keyval == GDK_KEY_End
                      ? (gint)items->len - 1
                      : ((gint)i + delta + (gint)items->len) % (gint)items->len;
      gtk_widget_grab_focus(g_ptr_array_index(items, next));
      break;
    }
  g_ptr_array_unref(items);
  return TRUE;
}
