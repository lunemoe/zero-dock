#include "zero-dock.h"
#include <X11/extensions/Xcomposite.h>
#include <cairo/cairo-xlib.h>
#include <math.h>
GdkPixbuf *zd_capture(ZdButton *b) {
  if (!b->window || b->closed)
    return NULL;
  if (!xfw_window_is_minimized(b->window)) {
    GdkDisplay *gd = gdk_display_get_default();
    Display *x = GDK_DISPLAY_XDISPLAY(gd);
    Window client = xfw_window_x11_get_xid(b->window), root, parent,
           *children = NULL;
    unsigned int n;
    Pixmap pix = 0;
    GdkPixbuf *result = NULL;
    gdk_x11_display_error_trap_push(gd);
    Window target = client;
    if (XQueryTree(x, client, &root, &parent, &children, &n) && parent != root)
      target = parent;
    if (children)
      XFree(children);
    XWindowAttributes attr;
    if (XGetWindowAttributes(x, target, &attr) &&
        attr.map_state == IsViewable && attr.width > 0 && attr.height > 0) {
      pix = XCompositeNameWindowPixmap(x, target);
      XSync(x, False);
      gint xerror = gdk_x11_display_error_trap_pop(gd);
      if (pix && xerror == 0) {
        gdk_x11_display_error_trap_push(gd);
        gint width = b->dock->preview_width,
             height =
                 MAX(1, (gint)round((double)attr.height * width / attr.width));
        // Bound tall windows without distorting their aspect ratio.
        if (height > 400) {
          width = MAX(1, (gint)round(width * 400. / height));
          height = 400;
        }
        cairo_surface_t *src = cairo_xlib_surface_create(
            x, pix, attr.visual, attr.width, attr.height);
        cairo_surface_t *dst =
            cairo_image_surface_create(CAIRO_FORMAT_ARGB32, width, height);
        cairo_t *cr = cairo_create(dst);
        cairo_scale(cr, (double)width / attr.width,
                    (double)height / attr.height);
        cairo_set_source_surface(cr, src, 0, 0);
        cairo_pattern_set_filter(cairo_get_source(cr), CAIRO_FILTER_BILINEAR);
        cairo_paint(cr);
        cairo_destroy(cr);
        cairo_surface_flush(dst);
        XSync(x, False);
        if (cairo_surface_status(dst) == CAIRO_STATUS_SUCCESS)
          result = gdk_pixbuf_get_from_surface(dst, 0, 0, width, height);
        cairo_surface_destroy(src);
        cairo_surface_destroy(dst);
        XFreePixmap(x, pix);
        if (gdk_x11_display_error_trap_pop(gd))
          g_clear_object(&result);
      } else {
        gdk_x11_display_error_trap_push(gd);
        if (pix)
          XFreePixmap(x, pix);
        gdk_x11_display_error_trap_pop_ignored(gd);
      }
    } else
      gdk_x11_display_error_trap_pop_ignored(gd);
    if (result) {
      g_set_object(&b->thumbnail, result);
      g_object_unref(result);
    }
  }
  return b->thumbnail ? g_object_ref(b->thumbnail) : NULL;
}
static void unlock(ZdDock *d) {
  if (d->autohide_blocked) {
    xfce_panel_plugin_block_autohide(d->plugin, FALSE);
    d->autohide_blocked = FALSE;
  }
}
void zd_preview_hide(ZdDock *d) {
  if (d->hover_id) {
    g_source_remove(d->hover_id);
    d->hover_id = 0;
  }
  if (d->preview_tick) {
    g_source_remove(d->preview_tick);
    d->preview_tick = 0;
  }
  if (d->leave_id) {
    g_source_remove(d->leave_id);
    d->leave_id = 0;
  }
  if (d->preview)
    gtk_widget_hide(d->preview);
  d->hover_button = NULL;
  unlock(d);
}
static gboolean contains(GtkWidget *w, gint x, gint y, gint pad) {
  if (!w || !gtk_widget_get_visible(w) || !gtk_widget_get_realized(w))
    return FALSE;
  gint ox, oy;
  GtkAllocation a;
  gtk_widget_get_allocation(w, &a);
  gdk_window_get_origin(gtk_widget_get_window(w), &ox, &oy);
  if (!gtk_widget_get_has_window(w)) {
    ox += a.x;
    oy += a.y;
  }
  return x >= ox - pad && x < ox + a.width + pad && y >= oy - pad &&
         y < oy + a.height + pad;
}
static gboolean check_leave(gpointer data) {
  ZdDock *d = data;
  d->leave_id = 0;
  if (!d->hover_button)
    return G_SOURCE_REMOVE;
  GdkDevice *p = gdk_seat_get_pointer(
      gdk_display_get_default_seat(gdk_display_get_default()));
  gint x, y;
  gdk_device_get_position(p, NULL, &x, &y);
  if (!contains(d->hover_button->main, x, y, 4) &&
      !contains(d->preview, x, y, 4))
    zd_preview_hide(d);
  return G_SOURCE_REMOVE;
}
void zd_preview_maybe_leave(ZdDock *d) {
  if (!d->leave_id)
    d->leave_id = g_timeout_add(30, check_leave, d);
}
static gboolean popup_leave(GtkWidget *w, GdkEventCrossing *e, ZdDock *d) {
  (void)w;
  if (e->detail != GDK_NOTIFY_INFERIOR)
    zd_preview_maybe_leave(d);
  return FALSE;
}
static gboolean popup_enter(GtkWidget *w, GdkEventCrossing *e, ZdDock *d) {
  (void)w;
  (void)e;
  if (d->leave_id) {
    g_source_remove(d->leave_id);
    d->leave_id = 0;
  }
  return FALSE;
}
static gboolean image_click(GtkWidget *w, GdkEventButton *e, ZdDock *d) {
  (void)w;
  if (!d->hover_button)
    return FALSE;
  if (e->button == 1) {
    zd_activate(d->hover_button);
    zd_preview_hide(d);
    return TRUE;
  }
  if (e->button == 2) {
    zd_launch(d->hover_button);
    return TRUE;
  }
  return FALSE;
}
static void popup_mute(GtkButton *w, ZdDock *d) {
  (void)w;
  if (d->hover_button) {
    zd_audio_mute(d->hover_button);
    zd_volume_bubble(d->hover_button);
  }
}
static gboolean popup_scroll(GtkWidget *w, GdkEventScroll *e, ZdDock *d) {
  (void)w;
  if (d->hover_button)
    zd_audio_scroll(d->hover_button, e);
  return TRUE;
}
static GtkWidget *popup_new(ZdDock *d) {
  GtkWidget *p = gtk_window_new(GTK_WINDOW_POPUP);
  gtk_window_set_accept_focus(GTK_WINDOW(p), FALSE);
  gtk_window_set_resizable(GTK_WINDOW(p), FALSE);
  gtk_window_set_type_hint(GTK_WINDOW(p), GDK_WINDOW_TYPE_HINT_TOOLTIP);
  gtk_style_context_add_class(gtk_widget_get_style_context(p), "zd-popup");
  gtk_style_context_add_provider(gtk_widget_get_style_context(p),
                                 GTK_STYLE_PROVIDER(d->css),
                                 GTK_STYLE_PROVIDER_PRIORITY_APPLICATION);
  xfce_panel_plugin_take_window(d->plugin, GTK_WINDOW(p));
  return p;
}
static void position_popup(ZdDock *d, GtkWidget *popup, GtkWidget *anchor) {
  GtkWidget *top = gtk_widget_get_toplevel(anchor);
  gint ax, ay, ox, oy;
  GtkAllocation allocation;
  gtk_widget_translate_coordinates(anchor, top, 0, 0, &ax, &ay);
  gdk_window_get_origin(gtk_widget_get_window(top), &ox, &oy);
  gtk_widget_get_allocation(anchor, &allocation);
  ax += ox;
  ay += oy;
  GtkRequisition natural;
  gtk_widget_get_preferred_size(popup, NULL, &natural);
  gint width = natural.width, height = natural.height;
  GdkMonitor *monitor = gdk_display_get_monitor_at_point(
      gdk_display_get_default(), ax + allocation.width / 2,
      ay + allocation.height / 2);
  GdkRectangle bounds;
  gdk_monitor_get_geometry(monitor, &bounds);
  gint x, y;
  if (d->orientation == GTK_ORIENTATION_HORIZONTAL) {
    x = ax + (allocation.width - width) / 2;
    y = ay + allocation.height + 3;
    if (ay + allocation.height / 2 > bounds.y + bounds.height / 2)
      y = ay - height - 3;
  } else {
    y = ay + (allocation.height - height) / 2;
    x = ax + allocation.width + 3;
    if (ax + allocation.width / 2 > bounds.x + bounds.width / 2)
      x = ax - width - 3;
  }
  x = CLAMP(x, bounds.x + 3,
            MAX(bounds.x + 3, bounds.x + bounds.width - width - 3));
  y = CLAMP(y, bounds.y + 3,
            MAX(bounds.y + 3, bounds.y + bounds.height - height - 3));
  gtk_window_resize(GTK_WINDOW(popup), width, height);
  gtk_window_move(GTK_WINDOW(popup), x, y);
}
void zd_preview_update_audio(ZdDock *d) {
  if (!d->preview || !d->hover_button)
    return;
  ZdButton *b = d->hover_button;
  b->audio = zd_audio_status(b);
  gtk_widget_set_sensitive(d->preview_sound, b->audio.present);
  gtk_button_set_image(GTK_BUTTON(d->preview_sound),
                       gtk_image_new_from_icon_name(
                           b->audio.muted ? "audio-volume-muted-symbolic"
                                          : "audio-volume-high-symbolic",
                           GTK_ICON_SIZE_BUTTON));
  gtk_widget_set_tooltip_text(d->preview_sound,
                              b->audio.muted ? "取消应用静音" : "应用静音");
}
static void update_preview(ZdDock *d) {
  ZdButton *b = d->hover_button;
  if (!b || !b->window)
    return;
  GdkPixbuf *p = zd_capture(b);
  if (p) {
    gtk_image_set_from_pixbuf(GTK_IMAGE(d->preview_image), p);
    g_object_unref(p);
    gtk_label_set_text(GTK_LABEL(d->preview_status),
                       xfw_window_is_minimized(b->window)
                           ? "已最小化 · 最后一帧"
                           : "点击预览切换窗口");
  } else {
    gtk_image_set_from_pixbuf(GTK_IMAGE(d->preview_image), b->icon);
    gtk_label_set_text(GTK_LABEL(d->preview_status),
                       "暂无可用画面 · 点击恢复窗口");
  }
  gtk_label_set_text(GTK_LABEL(d->preview_title),
                     xfw_window_get_name(b->window));
  zd_preview_update_audio(d);
  if (gtk_widget_get_visible(d->preview))
    position_popup(d, d->preview, b->main);
}
static gboolean preview_tick(gpointer data) {
  ZdDock *d = data;
  if (!d->hover_button)
    return G_SOURCE_REMOVE;
  update_preview(d);
  return G_SOURCE_CONTINUE;
}
static gboolean show_preview(gpointer data) {
  ZdDock *d = data;
  d->hover_id = 0;
  if (!d->hover_button || !d->previews || d->dragging ||
      xfw_screen_get_show_desktop(d->screen))
    return G_SOURCE_REMOVE;
  if (!d->preview) {
    d->preview = popup_new(d);
    GtkWidget *box = gtk_box_new(GTK_ORIENTATION_VERTICAL, 6),
              *image_box = gtk_event_box_new(),
              *row = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 6);
    gtk_container_add(GTK_CONTAINER(d->preview), box);
    gtk_container_set_border_width(GTK_CONTAINER(box), 6);
    d->preview_image = gtk_image_new();
    gtk_widget_set_size_request(image_box, d->preview_width, 140);
    gtk_container_add(GTK_CONTAINER(image_box), d->preview_image);
    gtk_box_pack_start(GTK_BOX(box), image_box, FALSE, FALSE, 0);
    d->preview_title = gtk_label_new("");
    gtk_label_set_ellipsize(GTK_LABEL(d->preview_title), PANGO_ELLIPSIZE_END);
    gtk_label_set_max_width_chars(GTK_LABEL(d->preview_title), 35);
    gtk_label_set_xalign(GTK_LABEL(d->preview_title), 0);
    gtk_box_pack_start(GTK_BOX(row), d->preview_title, TRUE, TRUE, 0);
    d->preview_sound = gtk_button_new();
    gtk_box_pack_end(GTK_BOX(row), d->preview_sound, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(box), row, FALSE, FALSE, 0);
    d->preview_status = gtk_label_new("");
    gtk_box_pack_start(GTK_BOX(box), d->preview_status, FALSE, FALSE, 0);
    gtk_widget_add_events(image_box, GDK_BUTTON_PRESS_MASK);
    g_signal_connect(image_box, "button-press-event", G_CALLBACK(image_click),
                     d);
    g_signal_connect(d->preview_sound, "clicked", G_CALLBACK(popup_mute), d);
    gtk_widget_add_events(d->preview_sound,
                          GDK_SCROLL_MASK | GDK_SMOOTH_SCROLL_MASK);
    g_signal_connect(d->preview_sound, "scroll-event", G_CALLBACK(popup_scroll),
                     d);
    g_signal_connect(d->preview, "enter-notify-event", G_CALLBACK(popup_enter),
                     d);
    g_signal_connect(d->preview, "leave-notify-event", G_CALLBACK(popup_leave),
                     d);
  }
  update_preview(d);
  gtk_widget_show_all(d->preview);
  position_popup(d, d->preview, d->hover_button->main);
  xfce_panel_plugin_block_autohide(d->plugin, TRUE);
  d->autohide_blocked = TRUE;
  d->preview_tick = g_timeout_add(650, preview_tick, d);
  return G_SOURCE_REMOVE;
}
void zd_preview_schedule(ZdButton *b) {
  ZdDock *d = b->dock;
  if (d->hover_button == b) {
    if (d->leave_id) {
      g_source_remove(d->leave_id);
      d->leave_id = 0;
    }
    return;
  }
  zd_preview_hide(d);
  if (b->pinned || !d->previews || d->dragging)
    return;
  d->hover_button = b;
  d->hover_id = g_timeout_add(350, show_preview, d);
}
static gboolean bubble_hide(gpointer p) {
  ZdDock *d = p;
  d->bubble_id = 0;
  if (d->bubble)
    gtk_widget_hide(d->bubble);
  return G_SOURCE_REMOVE;
}
void zd_volume_bubble(ZdButton *b) {
  ZdDock *d = b->dock;
  if (!d->bubble) {
    d->bubble = popup_new(d);
    GtkWidget *box = gtk_box_new(GTK_ORIENTATION_VERTICAL, 5);
    gtk_container_set_border_width(GTK_CONTAINER(box), 8);
    gtk_widget_set_size_request(box, 150, -1);
    gtk_container_add(GTK_CONTAINER(d->bubble), box);
    d->bubble_text = gtk_label_new("");
    d->bubble_bar = gtk_progress_bar_new();
    gtk_box_pack_start(GTK_BOX(box), d->bubble_text, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(box), d->bubble_bar, FALSE, FALSE, 0);
  }
  ZdAudioStatus a = zd_audio_status(b);
  gchar *s = g_strdup_printf(a.muted ? "应用已静音 · %u%%" : "应用音量 %u%%",
                             a.percent);
  gtk_label_set_text(GTK_LABEL(d->bubble_text), s);
  g_free(s);
  gtk_progress_bar_set_fraction(GTK_PROGRESS_BAR(d->bubble_bar),
                                MIN(a.percent / 200., 1.));
  gtk_widget_show_all(d->bubble);
  position_popup(d, d->bubble, b->main);
  if (d->bubble_id)
    g_source_remove(d->bubble_id);
  d->bubble_id = g_timeout_add(1200, bubble_hide, d);
}
void zd_popups_dispose(ZdDock *d) {
  zd_preview_hide(d);
  if (d->bubble_id)
    g_source_remove(d->bubble_id);
  if (d->preview)
    gtk_widget_destroy(d->preview);
  if (d->bubble)
    gtk_widget_destroy(d->bubble);
  d->preview = d->bubble = NULL;
}
