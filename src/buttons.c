#include "zero-dock.h"
#include <math.h>

/* XEmbed external plugins cannot rely on GTK_TARGET_SAME_APP.
 * Received keys are resolved against this instance; no pointer is trusted. */
static const GtkTargetEntry targets[] = {
    {"application/x-zero-dock-button", 0, 1}, {"text/uri-list", 0, 2}};
void zd_activate(ZdButton *b) {
  if (!b->window || b->closed)
    return;
  GError *e = NULL;
  XfwWorkspace *ws = xfw_window_get_workspace(b->window);
  if (ws && !(xfw_workspace_get_state(ws) & XFW_WORKSPACE_STATE_ACTIVE))
    xfw_workspace_activate(ws, NULL);
  if (xfw_window_is_minimized(b->window))
    xfw_window_set_minimized(b->window, FALSE, NULL);
  xfw_window_activate(b->window, NULL, zd_timestamp(b->dock), &e);
  if (e) {
    g_warning("Zero Dock activation: %s", e->message);
    g_error_free(e);
  }
}
void zd_minimize(ZdButton *b) {
  if (!b->window || b->closed)
    return;
  GdkPixbuf *p = zd_capture(b);
  g_clear_object(&p);
  xfw_window_set_minimized(b->window, TRUE, NULL);
}
void zd_toggle(ZdButton *b) {
  if (!b->window && b->pinned) {
    if (!b->launching)
      zd_launch(b);
    return;
  }
  if (b->dock->left_action == 0 && b->window && xfw_window_is_active(b->window))
    zd_minimize(b);
  else
    zd_activate(b);
}
void zd_cycle(ZdDock *d, gint delta) {
  GPtrArray *a = g_ptr_array_new();
  gint active = -1;
  for (GList *l = d->buttons; l; l = l->next) {
    ZdButton *b = l->data;
    if (b->window && b->eligible &&
        (d->all_workspaces || zd_window_in_workspace(b->window))) {
      if (xfw_window_is_active(b->window))
        active = a->len;
      g_ptr_array_add(a, b);
    }
  }
  if (a->len) {
    gint i = active < 0 ? (delta > 0 ? 0 : (gint)a->len - 1)
                        : (active + delta + (gint)a->len) % (gint)a->len;
    zd_activate(g_ptr_array_index(a, i));
  }
  g_ptr_array_free(a, TRUE);
}
static gboolean draw(GtkWidget *w, cairo_t *cr, ZdButton *b) {
  GtkAllocation a;
  gtk_widget_get_allocation(w, &a);
  gdouble width = a.width, height = a.height;
  gboolean active = b->window && xfw_window_is_active(b->window),
           min = b->window && xfw_window_is_minimized(b->window),
           urgent = b->window && xfw_window_is_urgent(b->window);
  GdkRGBA accent = {.red = .69, .green = .43, .blue = 1, .alpha = 1};
  gtk_style_context_lookup_color(gtk_widget_get_style_context(w),
                                 "theme_selected_bg_color", &accent);
  if (active) {
    cairo_set_source_rgba(cr, accent.red, accent.green, accent.blue, .20);
    cairo_rectangle(cr, 2, 2, width - 4, height - 4);
    cairo_fill(cr);
  }
  if (b->icon) {
    gdouble scale = MAX(1, b->icon_scale),
            x = (width - gdk_pixbuf_get_width(b->icon) / scale) / 2,
            y = (height - gdk_pixbuf_get_height(b->icon) / scale) / 2;
    cairo_save(cr);
    cairo_translate(cr, x, y);
    cairo_scale(cr, 1 / scale, 1 / scale);
    gdk_cairo_set_source_pixbuf(cr, b->icon, 0, 0);
    cairo_paint_with_alpha(cr, min ? .45 : 1);
    cairo_restore(cr);
  }
  if (b->launching) {
    gdouble angle = (g_get_monotonic_time() % 1000000) / 1000000. * 2 * G_PI;
    cairo_set_source_rgba(cr, accent.red, accent.green, accent.blue, .9);
    cairo_set_line_width(cr, 2);
    cairo_arc(cr, width / 2, height / 2, MIN(width, height) / 2 - 3, angle,
              angle + G_PI * 1.3);
    cairo_stroke(cr);
  } else if (b->launch_error || (b->pinned && !b->app)) {
    cairo_set_source_rgb(cr, 1, .65, .2);
    cairo_arc(cr, width - 8, height - 8, 4, 0, 2 * G_PI);
    cairo_fill(cr);
  }
  if (active) {
    gdk_cairo_set_source_rgba(cr, &accent);
    if (b->dock->orientation == GTK_ORIENTATION_HORIZONTAL)
      cairo_rectangle(cr, width * .25, height - 3, width * .5, 3);
    else
      cairo_rectangle(cr, width - 3, height * .25, 3, height * .5);
    cairo_fill(cr);
  }
  if (min) {
    cairo_set_source_rgb(cr, .7, .7, .75);
    cairo_arc(cr, width / 2, height - 4, 2, 0, 2 * G_PI);
    cairo_fill(cr);
  }
  if (urgent) {
    cairo_set_source_rgb(cr, 1, .75, .18);
    cairo_set_line_width(cr, 2);
    cairo_rectangle(cr, 2, 2, width - 4, height - 4);
    cairo_stroke(cr);
  }
  if (b->dock->numbers && b->number) {
    cairo_set_source_rgba(cr, .15, .15, .2, .95);
    cairo_arc(cr, 9, 9, 8, 0, 2 * G_PI);
    cairo_fill(cr);
    gchar *s = g_strdup_printf("%u", b->number);
    PangoLayout *p = gtk_widget_create_pango_layout(w, s);
    PangoFontDescription *f = pango_font_description_from_string("Sans Bold 8");
    pango_layout_set_font_description(p, f);
    gint tw, th;
    pango_layout_get_pixel_size(p, &tw, &th);
    cairo_set_source_rgb(cr, 1, 1, 1);
    cairo_move_to(cr, 9 - tw / 2., 9 - th / 2.);
    pango_cairo_show_layout(cr, p);
    pango_font_description_free(f);
    g_object_unref(p);
    g_free(s);
  }
  if (b->dock->insert_button == b) {
    cairo_set_source_rgb(cr, .25, .65, 1);
    if (b->dock->orientation == GTK_ORIENTATION_HORIZONTAL)
      cairo_rectangle(cr, b->dock->insert_after ? width - 3 : 0, 0, 3, height);
    else
      cairo_rectangle(cr, 0, b->dock->insert_after ? height - 3 : 0, width, 3);
    cairo_fill(cr);
  }
  return FALSE;
}
static gboolean pressed(GtkWidget *w, GdkEventButton *e, ZdButton *b) {
  (void)w;
  if (e->button == 3) {
    zd_preview_hide(b->dock);
    if (b->window)
      zd_window_menu(b, (GdkEvent *)e);
    else
      zd_pin_menu(b, (GdkEvent *)e);
    return TRUE;
  }
  if (e->button == 2) {
    if (b->dock->middle_action == 0)
      zd_launch(b);
    else if (b->dock->middle_action == 1)
      zd_close_window(b);
    return TRUE;
  }
  return FALSE;
}
static void clicked(GtkButton *w, ZdButton *b) {
  (void)w;
  if (!b->dock->dragging)
    zd_toggle(b);
}
void zd_close_window(ZdButton *b) {
  if (!b->window || b->closed)
    return;
  XfwWindow *window = g_object_ref(b->window);
  guint32 time = zd_timestamp(b->dock);
  zd_preview_hide(b->dock);
  xfw_window_close(window, time, NULL);
  g_object_unref(window);
}
static gboolean key_press(GtkWidget *widget, GdkEventKey *event, ZdButton *b) {
  if (event->keyval == GDK_KEY_Menu ||
      (event->keyval == GDK_KEY_F10 && (event->state & GDK_SHIFT_MASK))) {
    if (b->window)
      zd_window_menu(b, (GdkEvent *)event);
    else
      zd_pin_menu(b, (GdkEvent *)event);
    return TRUE;
  }
  if ((event->keyval == GDK_KEY_Return || event->keyval == GDK_KEY_KP_Enter) &&
      (event->state & GDK_CONTROL_MASK)) {
    zd_launch(b);
    return TRUE;
  }
  return zd_focus_key(widget, event, b->dock);
}
static gint scroll_steps(GdkEventScroll *e, gdouble *accumulator) {
  if (e->direction == GDK_SCROLL_UP || e->direction == GDK_SCROLL_LEFT) {
    *accumulator = 0;
    return 1;
  }
  if (e->direction == GDK_SCROLL_DOWN || e->direction == GDK_SCROLL_RIGHT) {
    *accumulator = 0;
    return -1;
  }
  gdouble dx = 0, dy = 0;
  if (!gdk_event_get_scroll_deltas((GdkEvent *)e, &dx, &dy))
    return 0;
  *accumulator -= fabs(dy) >= fabs(dx) ? dy : dx;
  gint steps = CLAMP(
      (gint)copysign(floor(fabs(*accumulator) + 1e-6), *accumulator), -32, 32);
  *accumulator -= steps;
  return steps;
}
static gboolean over_sound(GtkWidget *w, GdkEventScroll *e, ZdButton *b) {
  if (!gtk_widget_get_visible(b->sound))
    return FALSE;
  gint x, y;
  GtkAllocation a;
  gtk_widget_get_allocation(b->sound, &a);
  GtkWidget *top = gtk_widget_get_toplevel(b->sound);
  if (gtk_widget_get_realized(top) &&
      gtk_widget_translate_coordinates(b->sound, top, 0, 0, &x, &y)) {
    gint ox, oy;
    gdk_window_get_origin(gtk_widget_get_window(top), &ox, &oy);
    if (e->x_root != 0 || e->y_root != 0)
      return e->x_root >= ox + x && e->x_root < ox + x + a.width &&
             e->y_root >= oy + y && e->y_root < oy + y + a.height;
  }
  if (!gtk_widget_translate_coordinates(b->sound, w, 0, 0, &x, &y))
    return FALSE;
  return e->x >= x && e->x < x + a.width && e->y >= y && e->y < y + a.height;
}
static gboolean scroll(GtkWidget *w, GdkEventScroll *e, ZdButton *b) {
  gboolean audio = over_sound(w, e, b);
  if (audio) {
    zd_audio_scroll(b, e);
    return TRUE;
  }
  if (!b->dock->scroll_windows)
    return FALSE;
  gint steps = scroll_steps(e, &b->window_scroll);
  if (steps) {
    zd_cycle(b->dock, -steps);
  }
  return TRUE;
}
static gboolean sound_press(GtkWidget *w, GdkEventButton *e, ZdButton *b) {
  (void)w;
  (void)b;
  return e->button != 1;
}
static void sound_click(GtkButton *w, ZdButton *b) {
  (void)w;
  zd_audio_mute(b);
  zd_volume_bubble(b);
}
void zd_audio_scroll(ZdButton *b, GdkEventScroll *e) {
  if (e->time && b->last_audio_time == e->time)
    return;
  b->last_audio_time = e->time;
  gint s = scroll_steps(e, &b->audio_scroll);
  if (s) {
    zd_audio_volume(b, s);
    zd_volume_bubble(b);
  }
}
static gboolean sound_scroll(GtkWidget *w, GdkEventScroll *e, ZdButton *b) {
  (void)w;
  zd_audio_scroll(b, e);
  return TRUE;
}
static gboolean enter(GtkWidget *w, GdkEventCrossing *e, ZdButton *b) {
  (void)w;
  if (e->detail == GDK_NOTIFY_INFERIOR)
    return FALSE;
  zd_preview_schedule(b);
  return FALSE;
}
static gboolean leave(GtkWidget *w, GdkEventCrossing *e, ZdButton *b) {
  (void)w;
  if (e->detail != GDK_NOTIFY_INFERIOR)
    zd_preview_maybe_leave(b->dock);
  return FALSE;
}
static void drag_begin(GtkWidget *w, GdkDragContext *c, ZdButton *b) {
  (void)w;
  b->dock->dragging = TRUE;
  b->dock->drag_button = b;
  zd_preview_hide(b->dock);
  if (b->icon)
    gtk_drag_set_icon_pixbuf(c, b->icon, 0, 0);
}
static void drag_end(GtkWidget *w, GdkDragContext *c, ZdButton *b) {
  (void)w;
  (void)c;
  b->dock->dragging = FALSE;
  b->dock->drag_button = NULL;
  b->dock->insert_button = NULL;
  zd_update_buttons(b->dock);
}
static void drag_data(GtkWidget *w, GdkDragContext *c, GtkSelectionData *s,
                      guint info, guint time, ZdButton *b) {
  (void)w;
  (void)c;
  (void)time;
  if (info == 1)
    gtk_selection_data_set(s, gtk_selection_data_get_target(s), 8,
                           (const guchar *)b->key, strlen(b->key));
}
static gboolean drag_motion(GtkWidget *w, GdkDragContext *c, gint x, gint y,
                            guint time, ZdButton *b) {
  GtkAllocation a;
  gtk_widget_get_allocation(w, &a);
  b->dock->insert_button = b;
  b->dock->insert_after = b->dock->orientation == GTK_ORIENTATION_HORIZONTAL
                              ? x > a.width / 2
                              : y > a.height / 2;
  zd_update_buttons(b->dock);
  gdk_drag_status(c, b->dock->drag_button ? GDK_ACTION_MOVE : GDK_ACTION_COPY,
                  time);
  return TRUE;
}
static void drag_leave(GtkWidget *w, GdkDragContext *c, guint t, ZdButton *b) {
  (void)w;
  (void)c;
  (void)t;
  b->dock->insert_button = NULL;
  zd_update_buttons(b->dock);
}
static gboolean import_uris(ZdDock *d, GtkSelectionData *s) {
  gchar **uris = gtk_selection_data_get_uris(s);
  gboolean ok = FALSE;
  for (guint i = 0; uris && uris[i]; i++) {
    gchar *path = g_filename_from_uri(uris[i], NULL, NULL);
    if (path && g_str_has_suffix(path, ".desktop")) {
      GDesktopAppInfo *a = g_desktop_app_info_new_from_filename(path);
      if (a) {
        zd_pin_app(d, a);
        g_object_unref(a);
        ok = TRUE;
      }
    }
    g_free(path);
  }
  g_strfreev(uris);
  return ok;
}
static void received(GtkWidget *w, GdkDragContext *c, gint x, gint y,
                     GtkSelectionData *s, guint info, guint time, ZdButton *b) {
  (void)w;
  (void)x;
  (void)y;
  ZdDock *d = b->dock;
  gboolean ok = FALSE;
  if (info == 1 && gtk_selection_data_get_length(s) > 0) {
    gchar *key = g_strndup((const gchar *)gtk_selection_data_get_data(s),
                           gtk_selection_data_get_length(s));
    for (GList *l = d->buttons; l; l = l->next) {
      ZdButton *src = l->data;
      if (!strcmp(src->key, key)) {
        zd_move_button(d, src, b, d->insert_after);
        ok = TRUE;
        break;
      }
    }
    g_free(key);
  } else if (info == 2)
    ok = import_uris(d, s);
  d->insert_button = NULL;
  gtk_drag_finish(c, ok, FALSE, time);
  zd_update_buttons(d);
}
static void box_received(GtkWidget *w, GdkDragContext *c, gint x, gint y,
                         GtkSelectionData *s, guint info, guint time,
                         ZdDock *d) {
  (void)w;
  (void)x;
  (void)y;
  gtk_drag_finish(c, info == 2 && import_uris(d, s), FALSE, time);
}
static ZdButton *create(ZdDock *d) {
  ZdButton *b = g_new0(ZdButton, 1);
  b->dock = d;
  b->icon_dirty = TRUE;
  b->widget = gtk_overlay_new();
  b->main = gtk_button_new();
  b->drawing = gtk_drawing_area_new();
  gtk_container_add(GTK_CONTAINER(b->main), b->drawing);
  gtk_container_add(GTK_CONTAINER(b->widget), b->main);
  b->sound = gtk_button_new();
  gtk_widget_set_halign(b->sound, GTK_ALIGN_END);
  gtk_widget_set_valign(b->sound, GTK_ALIGN_START);
  gtk_widget_set_size_request(b->sound, 18, 18);
  gtk_style_context_add_class(gtk_widget_get_style_context(b->sound),
                              "zd-sound");
  gtk_overlay_add_overlay(GTK_OVERLAY(b->widget), b->sound);
  gtk_widget_set_no_show_all(b->sound, TRUE);
  gtk_style_context_add_provider(gtk_widget_get_style_context(b->main),
                                 GTK_STYLE_PROVIDER(d->css),
                                 GTK_STYLE_PROVIDER_PRIORITY_APPLICATION);
  gtk_style_context_add_provider(gtk_widget_get_style_context(b->sound),
                                 GTK_STYLE_PROVIDER(d->css),
                                 GTK_STYLE_PROVIDER_PRIORITY_APPLICATION);
  gtk_widget_add_events(b->main, GDK_SCROLL_MASK | GDK_SMOOTH_SCROLL_MASK |
                                     GDK_ENTER_NOTIFY_MASK |
                                     GDK_LEAVE_NOTIFY_MASK);
  gtk_widget_add_events(b->sound, GDK_SCROLL_MASK | GDK_SMOOTH_SCROLL_MASK);
  gtk_widget_add_events(b->widget, GDK_SCROLL_MASK | GDK_SMOOTH_SCROLL_MASK);
  g_signal_connect(b->drawing, "draw", G_CALLBACK(draw), b);
  g_signal_connect(b->main, "button-press-event", G_CALLBACK(pressed), b);
  gtk_widget_set_can_focus(b->main, TRUE);
  g_signal_connect(b->main, "key-press-event", G_CALLBACK(key_press), b);
  g_signal_connect(b->main, "clicked", G_CALLBACK(clicked), b);
  g_signal_connect(b->main, "scroll-event", G_CALLBACK(scroll), b);
  g_signal_connect(b->widget, "scroll-event", G_CALLBACK(scroll), b);
  g_signal_connect(b->widget, "enter-notify-event", G_CALLBACK(enter), b);
  g_signal_connect(b->main, "enter-notify-event", G_CALLBACK(enter), b);
  g_signal_connect(b->main, "leave-notify-event", G_CALLBACK(leave), b);
  g_signal_connect(b->sound, "button-press-event", G_CALLBACK(sound_press), b);
  g_signal_connect(b->sound, "clicked", G_CALLBACK(sound_click), b);
  g_signal_connect(b->sound, "scroll-event", G_CALLBACK(sound_scroll), b);
  gtk_drag_source_set(b->main, GDK_BUTTON1_MASK, targets, 1, GDK_ACTION_MOVE);
  gtk_drag_dest_set(b->main, GTK_DEST_DEFAULT_ALL, targets, 2,
                    GDK_ACTION_COPY | GDK_ACTION_MOVE);
  g_signal_connect(b->main, "drag-begin", G_CALLBACK(drag_begin), b);
  g_signal_connect(b->main, "drag-end", G_CALLBACK(drag_end), b);
  g_signal_connect(b->main, "drag-data-get", G_CALLBACK(drag_data), b);
  g_signal_connect(b->main, "drag-motion", G_CALLBACK(drag_motion), b);
  g_signal_connect(b->main, "drag-leave", G_CALLBACK(drag_leave), b);
  g_signal_connect(b->main, "drag-data-received", G_CALLBACK(received), b);
  if (!g_object_get_data(G_OBJECT(d->box), "drop-ready")) {
    gtk_drag_dest_set(d->box, GTK_DEST_DEFAULT_ALL, targets + 1, 1,
                      GDK_ACTION_COPY);
    g_signal_connect(d->box, "drag-data-received", G_CALLBACK(box_received), d);
    g_object_set_data(G_OBJECT(d->box), "drop-ready", GINT_TO_POINTER(1));
  }
  gtk_box_pack_start(GTK_BOX(d->box), b->widget, FALSE, FALSE, 0);
  gtk_widget_show_all(b->widget);
  gtk_widget_set_no_show_all(b->widget, TRUE);
  d->buttons = g_list_append(d->buttons, b);
  return b;
}
ZdButton *zd_add_pin(ZdDock *d, const gchar *path) {
  if (!path || !g_path_is_absolute(path) || !g_str_has_suffix(path, ".desktop"))
    return NULL;
  GDesktopAppInfo *a = g_desktop_app_info_new_from_filename(path);
  for (GList *l = d->buttons; l; l = l->next) {
    ZdButton *b = l->data;
    if (b->pinned && !g_strcmp0(b->desktop, path)) {
      g_clear_object(&a);
      return b;
    }
  }
  ZdButton *b = create(d);
  b->pinned = TRUE;
  b->app = a;
  b->desktop = g_strdup(path);
  b->key = g_strconcat("pin:", path, NULL);
  d->app_generation++;
  return b;
}
ZdButton *zd_add_window(ZdDock *d, XfwWindow *w) {
  GDesktopAppInfo *app = zd_match_app(d, w);
  ZdButton *b = NULL;
  for (GList *l = d->buttons; l; l = l->next) {
    ZdButton *pin = l->data;
    if (pin->pinned && !pin->window && zd_app_equal(pin->app, app)) {
      b = pin;
      break;
    }
  }
  if (!b) {
    b = create(d);
    b->app = app;
    app = NULL;
    b->key = g_strdup_printf("window:%lu", xfw_window_x11_get_xid(w));
  }
  g_clear_object(&app);
  zd_button_attach_window(b, w);
  for (GList *l = d->buttons; l; l = l->next) {
    ZdButton *q = l->data;
    if (q->launching && zd_app_equal(q->app, b->app))
      zd_launch_complete(q);
  }
  return b;
}
static void class_changed(ZdButton *b) {
  b->match_dirty = TRUE;
  b->pid = 0;
  b->thumbnail_time = 0;
  zd_queue_refresh(b->dock);
}
static void icon_changed(ZdButton *b) {
  b->icon_dirty = TRUE;
  zd_queue_refresh(b->dock);
}
static GdkFilterReturn identity_event(GdkXEvent *event, GdkEvent *gdk_event,
                                      gpointer data) {
  (void)gdk_event;
  XEvent *xevent = event;
  ZdButton *b = data;
  if (xevent->type == PropertyNotify)
    for (guint i = 0; i < G_N_ELEMENTS(b->dock->identity_atoms); i++)
      if (xevent->xproperty.atom == b->dock->identity_atoms[i]) {
        class_changed(b);
        break;
      }
  return GDK_FILTER_CONTINUE;
}
void zd_button_attach_window(ZdButton *b, XfwWindow *w) {
  ZdDock *d = b->dock;
  b->window = g_object_ref(w);
  b->pid = zd_window_pid(w);
  b->icon_dirty = TRUE;
  b->match_dirty = FALSE;
  b->match_generation = d->app_generation;
  g_hash_table_insert(d->windows, w, b);
  const gchar *signals[] = {"state-changed", "name-changed",
                            "workspace-changed", "capabilities-changed"};
  for (guint i = 0; i < G_N_ELEMENTS(signals); i++)
    g_signal_connect_swapped(w, signals[i], G_CALLBACK(zd_queue_refresh), d);
  g_signal_connect_swapped(w, "class-changed", G_CALLBACK(class_changed), b);
  g_signal_connect_swapped(w, "icon-changed", G_CALLBACK(icon_changed), b);
  GdkDisplay *gd = gdk_display_get_default();
  gdk_x11_display_error_trap_push(gd);
  b->xwindow =
      gdk_x11_window_foreign_new_for_display(gd, xfw_window_x11_get_xid(w));
  if (b->xwindow) {
    gdk_window_set_events(b->xwindow, gdk_window_get_events(b->xwindow) |
                                          GDK_PROPERTY_CHANGE_MASK);
    gdk_window_add_filter(b->xwindow, identity_event, b);
  }
  gdk_x11_display_error_trap_pop_ignored(gd);
}
void zd_button_release_window(ZdButton *b) {
  ZdDock *d = b->dock;
  if (d->hover_button == b)
    zd_preview_hide(d);
  if (d->drag_button == b) {
    d->drag_button = NULL;
    d->dragging = FALSE;
  }
  if (d->insert_button == b)
    d->insert_button = NULL;
  if (d->menu)
    gtk_widget_destroy(d->menu);
  if (b->xwindow) {
    gdk_window_remove_filter(b->xwindow, identity_event, b);
    g_clear_object(&b->xwindow);
  }
  if (b->window) {
    g_hash_table_remove(d->windows, b->window);
    g_signal_handlers_disconnect_by_data(b->window, d);
    g_signal_handlers_disconnect_by_data(b->window, b);
    g_clear_object(&b->window);
  }
  g_clear_object(&b->thumbnail);
  b->audio = (ZdAudioStatus){0};
  b->number = 0;
  b->audio_scroll = b->window_scroll = 0;
  b->last_audio_time = 0;
  b->pid = 0;
  b->icon_dirty = TRUE;
}
void zd_button_free(ZdButton *b) {
  b->closed = TRUE;
  zd_button_release_window(b);
  gtk_widget_destroy(b->widget);
  g_clear_object(&b->app);
  g_clear_object(&b->icon);
  g_free(b->desktop);
  g_free(b->key);
  g_free(b->startup_id);
  g_free(b->launch_error);
  g_free(b);
}
static gboolean same_app(ZdButton *a, ZdButton *b) {
  if (a->app && b->app)
    return zd_app_equal(a->app, b->app);
  const gchar *const *aa = xfw_window_get_class_ids(a->window),
                     *const *bb = xfw_window_get_class_ids(b->window);
  return aa && bb && aa[0] && bb[0] && !g_ascii_strcasecmp(aa[0], bb[0]);
}
gboolean zd_window_in_workspace(XfwWindow *w) {
  if (xfw_window_is_pinned(w))
    return TRUE;
  XfwWorkspace *ws = xfw_window_get_workspace(w);
  return !ws || (xfw_workspace_get_state(ws) & XFW_WORKSPACE_STATE_ACTIVE);
}
void zd_update_audio_buttons(ZdDock *d) {
  if (d->disposing)
    return;
  for (GList *l = d->buttons; l; l = l->next) {
    ZdButton *b = l->data;
    ZdAudioStatus old = b->audio;
    b->audio = zd_audio_status(b);
    gtk_widget_set_visible(b->sound, b->audio.present &&
                                         (b->audio.playing || b->audio.muted));
    if (!gtk_button_get_image(GTK_BUTTON(b->sound)) ||
        old.muted != b->audio.muted) {
      GtkWidget *image = gtk_image_new_from_icon_name(
          b->audio.muted ? "audio-volume-muted-symbolic"
                         : "audio-volume-high-symbolic",
          GTK_ICON_SIZE_MENU);
      gtk_button_set_image(GTK_BUTTON(b->sound), image);
      gtk_widget_show(image);
      gtk_widget_set_tooltip_text(b->sound, b->audio.muted
                                                ? _("取消应用静音 · 滚轮调音量")
                                                : _("应用静音 · 滚轮调音量"));
    }
  }
  zd_preview_update_audio(d);
}
void zd_update_buttons(ZdDock *d) {
  zd_layout_update(d);
  for (GList *l = d->buttons; l; l = l->next) {
    ZdButton *b = l->data;
    guint scale = gtk_widget_get_scale_factor(d->box),
          pixels = d->icon_size * scale;
    if (b->icon_dirty || b->icon_pixels != pixels || b->icon_scale != scale) {
      g_clear_object(&b->icon);
      b->icon_dirty = FALSE;
      b->icon_pixels = pixels;
      b->icon_scale = scale;
      if (b->window) {
        GdkPixbuf *p = xfw_window_get_icon(b->window, d->icon_size,
                                           gtk_widget_get_scale_factor(d->box));
        if (p)
          b->icon =
              gdk_pixbuf_scale_simple(p, pixels, pixels, GDK_INTERP_BILINEAR);
      }
      if (!b->icon && b->app) {
        GIcon *i = g_app_info_get_icon(G_APP_INFO(b->app));
        GtkIconInfo *info = i ? gtk_icon_theme_lookup_by_gicon_for_scale(
                                    d->icon_theme, i, d->icon_size, scale,
                                    GTK_ICON_LOOKUP_FORCE_SIZE)
                              : NULL;
        if (info) {
          b->icon = gtk_icon_info_load_icon(info, NULL);
          g_object_unref(info);
        }
      }
      if (!b->icon)
        b->icon = gtk_icon_theme_load_icon_for_scale(
            d->icon_theme,
            b->pinned && !b->app ? "dialog-warning"
                                 : "application-x-executable",
            d->icon_size, scale, GTK_ICON_LOOKUP_FORCE_SIZE, NULL);
    }
    b->number = 0;
    if (b->window) {
      guint n = 0, order = 0;
      for (GList *k = d->buttons; k; k = k->next) {
        ZdButton *q = k->data;
        if (q->window && same_app(b, q) &&
            (d->all_workspaces || q->pinned ||
             zd_window_in_workspace(q->window))) {
          n++;
          if (q == b)
            order = n;
        }
      }
      if (n > 1)
        b->number = order;
    }
    gtk_widget_set_size_request(b->main, d->unit, d->unit);
    gtk_widget_set_size_request(b->drawing, d->unit, d->unit);
    const gchar *title = b->window ? xfw_window_get_name(b->window)
                         : b->app
                             ? g_app_info_get_display_name(G_APP_INFO(b->app))
                             : _("启动器文件已失效");
    gtk_widget_set_tooltip_text(b->main, b->launching      ? _("正在启动…")
                                         : b->launch_error ? b->launch_error
                                         : d->previews && b->window ? NULL
                                                                    : title);
    atk_object_set_name(gtk_widget_get_accessible(b->main), title);
    gtk_widget_queue_draw(b->drawing);
    if (b->window && gtk_widget_get_realized(b->main)) {
      GtkAllocation a;
      gtk_widget_get_allocation(b->main, &a);
      GdkRectangle r = {a.x, a.y, a.width, a.height};
      xfw_window_set_button_geometry(b->window, gtk_widget_get_window(b->main),
                                     &r, NULL);
    }
  }
  zd_update_audio_buttons(d);
}
