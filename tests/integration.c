#include "../src/zero-dock.h"
#include <X11/Xutil.h>
#include <X11/extensions/XTest.h>
#include <glib/gstdio.h>
#include <gtk/gtkx.h>
#include <libxfce4panel/xfce-panel-plugin-provider.h>
#include <signal.h>
#include <stdio.h>
#include <sys/resource.h>
#include <sys/wait.h>
#include <unistd.h>
static void spin(guint ms) {
  gint64 end = g_get_monotonic_time() + ms * 1000;
  do {
    while (g_main_context_iteration(NULL, FALSE))
      ;
    g_usleep(2000);
  } while (g_get_monotonic_time() < end);
}
static GtkWidget *fixture(const char *title) {
  GtkWidget *w = gtk_window_new(GTK_WINDOW_TOPLEVEL);
  gtk_window_set_title(GTK_WINDOW(w), title);
  gtk_window_set_default_size(GTK_WINDOW(w), 380, 220);
  GtkWidget *l = gtk_label_new("Zero Dock 图像测试\nFixture pixels / 预览缓存");
  GtkWidget *events = gtk_event_box_new();
  gtk_container_add(GTK_CONTAINER(events), l);
  gtk_container_add(GTK_CONTAINER(w), events);
  gtk_widget_realize(w);
  Display *x = GDK_DISPLAY_XDISPLAY(gdk_display_get_default());
  XClassHint h = {"zero-dock-fixture", "ZeroDockFixture"};
  XSetClassHint(x, GDK_WINDOW_XID(gtk_widget_get_window(w)), &h);
  gtk_widget_show_all(w);
  return w;
}
static XfcePanelPlugin *host(const char *rc, guint id, GtkWidget **out) {
  *out = gtk_window_new(GTK_WINDOW_TOPLEVEL);
  gtk_window_set_skip_taskbar_hint(GTK_WINDOW(*out), TRUE);
  gtk_window_set_accept_focus(GTK_WINDOW(*out), FALSE);
  gtk_window_set_default_size(GTK_WINDOW(*out), 600, 56);
  gtk_window_move(GTK_WINDOW(*out), 10, 700);
  XfcePanelPlugin *p =
      g_object_new(XFCE_TYPE_PANEL_PLUGIN, "name", "zero-dock", "unique-id", id,
                   "display-name", "Zero Dock Test", "comment", "test", NULL);
  g_object_set_data(G_OBJECT(p), "test-rc", (gpointer)rc);
  gtk_container_add(GTK_CONTAINER(*out), GTK_WIDGET(p));
  xfce_panel_plugin_provider_set_size(XFCE_PANEL_PLUGIN_PROVIDER(p), 48);
  zd_construct(p);
  gtk_widget_show_all(*out);
  return p;
}
static ZdButton *button(ZdDock *d, GtkWidget *w) {
  Window xid = GDK_WINDOW_XID(gtk_widget_get_window(w));
  for (GList *l = d->buttons; l; l = l->next) {
    ZdButton *b = l->data;
    if (b->window && xfw_window_x11_get_xid(b->window) == xid)
      return b;
  }
  return NULL;
}
static void point_at(GtkWidget *w, gint *rx, gint *ry) {
  GtkWidget *top = gtk_widget_get_toplevel(w);
  gint x, y, ox, oy;
  gtk_widget_translate_coordinates(w, top, 0, 0, &x, &y);
  gdk_window_get_origin(gtk_widget_get_window(top), &ox, &oy);
  GtkAllocation a;
  gtk_widget_get_allocation(w, &a);
  *rx = ox + x + a.width / 2;
  *ry = oy + y + a.height / 2;
}
static void mouse_at(GtkWidget *w) {
  gint x, y;
  point_at(w, &x, &y);
  Display *display = GDK_DISPLAY_XDISPLAY(gdk_display_get_default());
  XTestFakeMotionEvent(display, -1, x, y, CurrentTime);
  XFlush(display);
}
static void image_pressed(ZdDock *d) {
  GtkWidget *box = gtk_bin_get_child(GTK_BIN(d->preview));
  GList *children = gtk_container_get_children(GTK_CONTAINER(box));
  GtkWidget *imagebox = children->data;
  mouse_at(imagebox);
  spin(100);
  g_assert_true(gtk_widget_get_visible(d->preview));
  Display *x = GDK_DISPLAY_XDISPLAY(gdk_display_get_default());
  XTestFakeButtonEvent(x, 1, True, CurrentTime);
  XTestFakeButtonEvent(x, 1, False, CurrentTime);
  XFlush(x);
  spin(150);
  g_list_free(children);
}
static void drag_between(GtkWidget *source, GtkWidget *target) {
  gint sx, sy, tx, ty;
  point_at(source, &sx, &sy);
  point_at(target, &tx, &ty);
  tx -= 15;
  g_test_message("drag from %d,%d to %d,%d", sx, sy, tx, ty);
  Display *x = GDK_DISPLAY_XDISPLAY(gdk_display_get_default());
  XTestFakeMotionEvent(x, -1, sx, sy, CurrentTime);
  XFlush(x);
  spin(60);
  XTestFakeButtonEvent(x, 1, True, CurrentTime);
  XFlush(x);
  spin(60);
  for (guint i = 1; i <= 12; i++) {
    XTestFakeMotionEvent(x, -1, sx + (tx - sx) * (gint)i / 12,
                         sy + (ty - sy) * (gint)i / 12, CurrentTime);
    XFlush(x);
    spin(20);
  }
  XTestFakeButtonEvent(x, 1, False, CurrentTime);
  XFlush(x);
  spin(200);
}
static void evidence(GtkWidget *w, const char *name) {
  const char *dir = g_getenv("ZERO_DOCK_EVIDENCE_DIR");
  if (!dir)
    return;
  g_mkdir_with_parents(dir, 0700);
  GtkAllocation a;
  gtk_widget_get_allocation(w, &a);
  GdkPixbuf *p = gdk_pixbuf_get_from_window(gtk_widget_get_window(w), 0, 0,
                                            a.width, a.height);
  if (p) {
    gchar *path = g_build_filename(dir, name, NULL);
    gdk_pixbuf_save(p, path, "png", NULL, NULL);
    g_free(path);
    g_object_unref(p);
  }
}
static void uri_data(GtkWidget *w, GdkDragContext *c, GtkSelectionData *s,
                     guint info, guint time, const gchar *desktop) {
  (void)w;
  (void)c;
  (void)info;
  (void)time;
  gchar *uri = g_filename_to_uri(desktop, NULL, NULL);
  gchar *uris[] = {uri, NULL};
  gtk_selection_data_set_uris(s, uris);
  g_free(uri);
}
static gboolean plug_removed(GtkSocket *s, gpointer data) {
  (void)s;
  (void)data;
  return TRUE;
}
static void external(void) {
  const gchar *build = g_getenv("ZERO_DOCK_TEST_BUILD");
  gchar *library =
      g_build_filename(build ? build : ".", "libzero-dock.so", NULL);
  GtkWidget *f1 = fixture("External Red"), *f2 = fixture("External Green");
  GdkPixbuf *red = gdk_pixbuf_new(GDK_COLORSPACE_RGB, TRUE, 8, 32, 32),
            *green = gdk_pixbuf_new(GDK_COLORSPACE_RGB, TRUE, 8, 32, 32);
  gdk_pixbuf_fill(red, 0xdd3333ff);
  gdk_pixbuf_fill(green, 0x33dd33ff);
  gtk_window_set_icon(GTK_WINDOW(f1), red);
  gtk_window_set_icon(GTK_WINDOW(f2), green);
  g_object_unref(red);
  g_object_unref(green);
  spin(200);
  GtkWidget *host = gtk_window_new(GTK_WINDOW_TOPLEVEL),
            *box = gtk_box_new(GTK_ORIENTATION_VERTICAL, 3),
            *socket1 = gtk_socket_new(), *socket2 = gtk_socket_new();
  gtk_window_set_skip_taskbar_hint(GTK_WINDOW(host), TRUE);
  gtk_window_move(GTK_WINDOW(host), 10, 500);
  gtk_container_add(GTK_CONTAINER(host), box);
  gtk_widget_set_size_request(socket1, 520, 48);
  gtk_widget_set_size_request(socket2, 520, 48);
  gtk_box_pack_start(GTK_BOX(box), socket1, FALSE, FALSE, 0);
  gtk_box_pack_start(GTK_BOX(box), socket2, FALSE, FALSE, 0);
  g_signal_connect(socket1, "plug-removed", G_CALLBACK(plug_removed), NULL);
  g_signal_connect(socket2, "plug-removed", G_CALLBACK(plug_removed), NULL);
  gtk_widget_show_all(host);
  spin(100);
  gchar *sid1 = g_strdup_printf("%lu", gtk_socket_get_id(GTK_SOCKET(socket1))),
        *sid2 = g_strdup_printf("%lu", gtk_socket_get_id(GTK_SOCKET(socket2)));
  gchar *args1[] = {"/usr/lib/xfce4/panel/wrapper-2.0",
                    library,
                    "9091",
                    sid1,
                    "zero-dock",
                    "Zero Dock Test",
                    "isolated external test",
                    NULL};
  gchar *args2[] = {"/usr/lib/xfce4/panel/wrapper-2.0",
                    library,
                    "9092",
                    sid2,
                    "zero-dock",
                    "Zero Dock Test",
                    "isolated external test",
                    NULL};
  GPid one, two;
  gchar **wrapper_env = g_get_environ();
  const gchar *asan = g_getenv("ZERO_DOCK_ASAN_PRELOAD");
  if (asan)
    wrapper_env = g_environ_setenv(wrapper_env, "LD_PRELOAD", asan, TRUE);
  g_assert_true(g_spawn_async(NULL, args1, wrapper_env,
                              G_SPAWN_DO_NOT_REAP_CHILD, NULL, NULL, &one,
                              NULL));
  g_assert_true(g_spawn_async(NULL, args2, wrapper_env,
                              G_SPAWN_DO_NOT_REAP_CHILD, NULL, NULL, &two,
                              NULL));
  spin(800);
  g_assert_nonnull(gtk_socket_get_plug_window(GTK_SOCKET(socket1)));
  g_assert_nonnull(gtk_socket_get_plug_window(GTK_SOCKET(socket2)));
  g_assert_cmpint(one, !=, two);
  gint ox, oy;
  gdk_window_get_origin(gtk_widget_get_window(socket1), &ox, &oy);
  GtkAllocation allocation;
  gtk_widget_get_allocation(socket1, &allocation);
  g_assert_cmpint(allocation.width, >, 60);
  g_assert_cmpint(allocation.height, >, 20);
  gint cy = allocation.height / 2;
  GdkPixbuf *before =
      gdk_pixbuf_get_from_window(gtk_widget_get_window(socket1), 0, 0,
                                 allocation.width, allocation.height);
  evidence(socket1, "wrapper-before.png");
  g_test_message("wrapper size %d x %d, centre y %d", allocation.width,
                 allocation.height, cy);
  g_assert_nonnull(before);
  const guchar *pixel = gdk_pixbuf_get_pixels(before) +
                        cy * gdk_pixbuf_get_rowstride(before) +
                        14 * gdk_pixbuf_get_n_channels(before);
  g_assert_cmpint(pixel[0], >, pixel[1]);
  g_object_unref(before);
  Display *xd = GDK_DISPLAY_XDISPLAY(gdk_display_get_default());
  XTestFakeMotionEvent(xd, -1, ox + 44, oy + cy, CurrentTime);
  XFlush(xd);
  spin(80);
  XTestFakeButtonEvent(xd, 1, True, CurrentTime);
  XFlush(xd);
  spin(80);
  for (guint k = 1; k <= 12; k++) {
    XTestFakeMotionEvent(xd, -1, ox + 44 - 37 * (gint)k / 12, oy + cy,
                         CurrentTime);
    XFlush(xd);
    spin(20);
  }
  XTestFakeButtonEvent(xd, 1, False, CurrentTime);
  XFlush(xd);
  spin(250);
  GdkPixbuf *after =
      gdk_pixbuf_get_from_window(gtk_widget_get_window(socket1), 0, 0,
                                 allocation.width, allocation.height);
  g_assert_nonnull(after);
  pixel = gdk_pixbuf_get_pixels(after) + cy * gdk_pixbuf_get_rowstride(after) +
          14 * gdk_pixbuf_get_n_channels(after);
  g_assert_cmpint(pixel[1], >, pixel[0]);
  g_object_unref(after);
  g_test_message("PASS real drag inside the XEmbed external wrapper changes "
                 "red/green window order");
  // Deliberately terminate only our isolated wrapper, leaving the host and peer
  // alive.
  kill(one, SIGTERM);
  waitpid(one, NULL, 0);
  g_spawn_close_pid(one);
  spin(150);
  g_assert_true(gtk_widget_get_visible(host));
  g_assert_nonnull(gtk_socket_get_plug_window(GTK_SOCKET(socket2)));
  g_assert_cmpint(kill(two, 0), ==, 0);
  kill(two, SIGTERM);
  waitpid(two, NULL, 0);
  g_spawn_close_pid(two);
  spin(100);
  gtk_widget_destroy(host);
  gtk_widget_destroy(f1);
  gtk_widget_destroy(f2);
  spin(100);
  g_free(library);
  g_free(sid1);
  g_free(sid2);
  g_strfreev(wrapper_env);
  g_test_message("PASS two actual XFCE wrapper processes; losing one leaves "
                 "host and peer alive");
}
static void all(void) {
  gchar *dir = g_dir_make_tmp("zero-dock-integration-XXXXXX", NULL);
  g_assert_nonnull(dir);
  gchar *rc = g_build_filename(dir, "one.rc", NULL),
        *rc2 = g_build_filename(dir, "two.rc", NULL),
        *desktop = g_build_filename(dir, "fixture.desktop", NULL),
        *marker = g_build_filename(dir, "launched", NULL);
  gchar *entry = g_strdup_printf(
      "[Desktop Entry]\nType=Application\nName=Zero Dock "
      "Fixture\nExec=/usr/bin/touch "
      "%s\nIcon=utilities-terminal\nStartupWMClass=ZeroDockFixture\nActions="
      "Test;\n[Desktop Action Test]\nName=Test desktop "
      "action\nExec=/usr/bin/touch %s\n",
      marker, marker);
  g_assert_true(g_file_set_contents(desktop, entry, -1, NULL));
  g_free(entry);
  GtkWidget *f1 = fixture("ZeroDockFixture One"),
            *f2 = fixture("ZeroDockFixture Two");
  spin(500);
  GtkWidget *h, *h2;
  XfcePanelPlugin *p = host(rc, 9001, &h), *p2 = host(rc2, 9002, &h2);
  ZdDock *d = zd_get_dock(p), *d2 = zd_get_dock(p2);
  gtk_window_move(GTK_WINDOW(h2), 10, 550);
  spin(500);
  xfce_panel_plugin_provider_set_icon_size(XFCE_PANEL_PLUGIN_PROVIDER(p), 0);
  spin(50);
  g_assert_cmpuint(d->icon_size, ==, 40);
  xfce_panel_plugin_provider_set_icon_size(XFCE_PANEL_PLUGIN_PROVIDER(p), 36);
  spin(50);
  g_assert_cmpuint(d->icon_size, ==, 36);
  ZdButton *a = button(d, f1), *b = button(d, f2);
  g_assert_nonnull(a);
  g_assert_nonnull(b);
  g_assert_true(a != b);
  g_assert_cmpuint(g_hash_table_size(d->windows), ==, 2);
  g_assert_cmpuint(a->number, >, 0);
  g_assert_cmpuint(b->number, >, 0);
  g_assert_cmpuint(a->number, !=, b->number);
  g_test_message(
      "PASS two windows of one app remain separate, numbered, icon only");
  GtkWidget *uri_source = gtk_bin_get_child(GTK_BIN(f2));
  GtkTargetEntry uri_target = {"text/uri-list", 0, 2};
  gtk_drag_source_set(uri_source, GDK_BUTTON1_MASK, &uri_target, 1,
                      GDK_ACTION_COPY);
  g_signal_connect(uri_source, "drag-data-get", G_CALLBACK(uri_data), desktop);
  drag_between(uri_source, a->main);
  g_assert_cmpuint(g_list_length(d->buttons), ==, 2);
  g_assert_true(a->pinned);
  g_test_message(
      "PASS real desktop-file URI drag pins a running button in place");
  GDesktopAppInfo *app = g_desktop_app_info_new_from_filename(desktop);
  g_set_object(&a->app, app);
  g_set_object(&b->app, app);
  zd_pin_app(d, app);
  zd_pin_app(d, app);
  g_assert_cmpuint(g_list_length(d->buttons), ==, 2);
  ZdButton *pin = a;
  g_assert_true(pin->pinned);
  g_assert_cmpuint(g_list_length(d2->buttons), ==, 2);
  zd_move_button(d, pin, b, FALSE);
  g_assert_true(d->buttons->data == pin);
  zd_move_button(d, b, a, FALSE);
  g_assert_true(d->buttons->data == b);
  spin(150);
  evidence(h, "drag-before.png");
  drag_between(a->main, b->main);
  evidence(h, "drag-after.png");
  g_test_message("dragging=%d target=%p", d->dragging,
                 (void *)d->insert_button);
  g_assert_cmpint(g_list_index(d->buttons, a), <, g_list_index(d->buttons, b));
  g_test_message("PASS real mouse drag reorders independent buttons");
  zd_launch(pin);
  spin(150);
  g_assert_true(g_file_test(marker, G_FILE_TEST_EXISTS));
  g_test_message("PASS pin deduplication, launch, combined ordering, "
                 "per-instance isolation");
  zd_activate(a);
  spin(200);
  GdkPixbuf *pix = zd_capture(a);
  g_assert_nonnull(pix);
  g_assert_cmpint(gdk_pixbuf_get_width(pix), ==, 300);
  g_assert_cmpint(gdk_pixbuf_get_height(pix), >, 100);
  g_object_unref(pix);
  zd_minimize(a);
  spin(200);
  g_assert_true(xfw_window_is_minimized(a->window));
  pix = zd_capture(a);
  g_assert_nonnull(pix);
  g_object_unref(pix);
  zd_preview_schedule(a);
  spin(400);
  g_assert_nonnull(d->preview);
  g_assert_true(gtk_widget_get_visible(d->preview));
  g_assert_true(d->autohide_blocked);
  gint popup_x, popup_y;
  gdk_window_get_origin(gtk_widget_get_window(d->preview), &popup_x, &popup_y);
  GtkAllocation popup_alloc;
  gtk_widget_get_allocation(d->preview, &popup_alloc);
  g_assert_cmpint(popup_x, >=, 0);
  g_assert_cmpint(popup_y, >=, 0);
  g_assert_cmpint(popup_x + popup_alloc.width, <=, 1024);
  g_assert_cmpint(popup_y + popup_alloc.height, <=, 768);
  spin(700);
  g_assert_nonnull(a->thumbnail);
  evidence(d->preview, "preview.png");
  evidence(h, "dock.png");
  image_pressed(d);
  spin(200);
  g_assert_true(xfw_window_is_active(a->window));
  g_assert_false(xfw_window_is_minimized(a->window));
  g_assert_false(d->autohide_blocked);
  g_test_message("PASS composite frame capture, minimized cached preview, "
                 "click activation, autohide unlock");
  zd_preview_schedule(a);
  spin(400);
  zd_preview_hide(d);
  g_assert_false(gtk_widget_get_visible(d->preview));
  g_assert_null(d->hover_button);
  d->previews = FALSE;
  zd_update_buttons(d);
  g_assert_nonnull(gtk_widget_get_tooltip_text(a->main));
  d->previews = TRUE;
  GdkEvent *event = gdk_event_new(GDK_BUTTON_PRESS);
  event->button.window = g_object_ref(gtk_widget_get_window(a->main));
  event->button.button = 3;
  event->button.time = zd_timestamp(d);
  gdk_event_set_device(event, gdk_seat_get_pointer(gdk_display_get_default_seat(
                                  gdk_display_get_default())));
  zd_window_menu(a, event);
  g_assert_nonnull(d->menu);
  g_assert_true(gtk_widget_get_visible(d->menu));
  GList *items = gtk_container_get_children(GTK_CONTAINER(d->menu));
  g_assert_cmpuint(g_list_length(items), >=, 10);
  g_list_free(items);
  gtk_widget_destroy(d->menu);
  zd_pin_menu(pin, event);
  items = gtk_container_get_children(GTK_CONTAINER(d->menu));
  g_assert_cmpuint(g_list_length(items), >=, 6);
  g_list_free(items);
  gtk_widget_destroy(d->menu);
  gdk_event_free(event);
  g_test_message("PASS window/workspace menu, desktop actions menu and "
                 "tooltip fallback");
  gchar *av[] = {"/usr/bin/pacat", "--playback",
                 "--raw",          "--rate=8000",
                 "--channels=1",   "--format=s16le",
                 "--volume=0",     "--stream-name=ZeroDockIntegration",
                 "/dev/zero",      NULL};
  GPid pid;
  g_assert_true(g_spawn_async(NULL, av, NULL, G_SPAWN_DO_NOT_REAP_CHILD, NULL,
                              NULL, &pid, NULL));
  spin(800);
  ZdAudioStatus s = zd_audio_status(a);
  g_assert_true(s.present);
  g_assert_true(s.playing);
  g_assert_false(s.muted);
  g_assert_true(gtk_widget_get_visible(a->sound));
  zd_activate(a);
  spin(200);
  mouse_at(a->sound);
  spin(100);
  XfwWindow *active_before_scroll = xfw_screen_get_active_window(d->screen);
  Display *scroll_display = GDK_DISPLAY_XDISPLAY(gdk_display_get_default());
  XTestFakeButtonEvent(scroll_display, 4, True, CurrentTime);
  XTestFakeButtonEvent(scroll_display, 4, False, CurrentTime);
  XFlush(scroll_display);
  spin(150);
  g_assert_cmpuint(zd_audio_status(a).percent, ==, 5);
  g_assert_true(xfw_screen_get_active_window(d->screen) ==
                active_before_scroll);
  zd_audio_volume(a, -1);
  spin(150);
  g_test_message(
      "PASS physical wheel over speaker badge changes only app volume");
  GdkEvent *smooth = gdk_event_new(GDK_SCROLL);
  smooth->scroll.direction = GDK_SCROLL_SMOOTH;
  smooth->scroll.delta_y = -0.125;
  gint badge_x, badge_y;
  GtkAllocation badge;
  gtk_widget_get_allocation(a->sound, &badge);
  g_assert_true(gtk_widget_translate_coordinates(a->sound, a->main, 0, 0,
                                                 &badge_x, &badge_y));
  smooth->scroll.x = badge_x + badge.width / 2;
  smooth->scroll.y = badge_y + badge.height / 2;
  gint badge_root_x, badge_root_y;
  point_at(a->sound, &badge_root_x, &badge_root_y);
  smooth->scroll.x_root = badge_root_x;
  smooth->scroll.y_root = badge_root_y;
  // XEmbed can deliver coordinates relative to a different event window.
  smooth->scroll.x = smooth->scroll.y = -999;
  for (guint i = 0; i < 7; i++) {
    gboolean handled = FALSE;
    g_signal_emit_by_name(a->main, "scroll-event", smooth, &handled);
    g_assert_true(handled);
  }
  spin(100);
  g_assert_cmpuint(zd_audio_status(a).percent, ==, 0);
  gboolean handled = FALSE;
  g_signal_emit_by_name(a->main, "scroll-event", smooth, &handled);
  g_assert_true(handled);
  spin(150);
  g_assert_cmpuint(zd_audio_status(a).percent, ==, 5);
  g_assert_true(xfw_screen_get_active_window(d->screen) ==
                active_before_scroll);
  gdk_event_free(smooth);
  zd_audio_volume(a, -1);
  spin(150);
  g_test_message("PASS eight fractional scroll events accumulate one volume "
                 "step through parent-button badge hit testing");
  g_assert_nonnull(d->input);
  g_assert_false(zd_input_delta(d, -100, -100, 0, -1, 1234));
  g_assert_true(zd_input_delta(d, badge_root_x, badge_root_y, 0, -1, 1234));
  // A matching GTK event must not apply the same physical tick twice.
  smooth = gdk_event_new(GDK_SCROLL);
  smooth->scroll.direction = GDK_SCROLL_SMOOTH;
  smooth->scroll.delta_y = -1;
  smooth->scroll.time = 1234;
  zd_audio_scroll(a, &smooth->scroll);
  gdk_event_free(smooth);
  spin(150);
  g_assert_cmpuint(zd_audio_status(a).percent, ==, 5);
  zd_audio_volume(a, -1);
  spin(150);
  g_test_message(
      "PASS XInput2 screen hit testing and raw/GTK duplicate suppression");
  zd_preview_hide(d);
  zd_preview_schedule(a);
  spin(400);
  g_assert_true(gtk_widget_get_visible(d->preview));
  mouse_at(d->preview_sound);
  spin(100);
  XTestFakeButtonEvent(scroll_display, 4, True, CurrentTime);
  XTestFakeButtonEvent(scroll_display, 4, False, CurrentTime);
  XFlush(scroll_display);
  spin(200);
  g_assert_cmpuint(zd_audio_status(a).percent, ==, 5);
  zd_audio_volume(a, -1);
  spin(150);
  zd_preview_hide(d);
  g_test_message("PASS physical wheel over preview speaker changes app volume");
  zd_audio_mute(a);
  spin(150);
  g_assert_true(zd_audio_status(a).muted);
  zd_audio_volume(a, 1);
  spin(150);
  g_assert_cmpuint(zd_audio_status(a).percent, ==, 5);
  zd_audio_volume(a, 100);
  spin(150);
  g_assert_cmpuint(zd_audio_status(a).percent, ==, 200);
  zd_volume_bubble(a);
  g_assert_true(gtk_widget_get_visible(d->bubble));
  spin(1300);
  g_assert_false(gtk_widget_get_visible(d->bubble));
  zd_audio_volume(a, -100);
  zd_audio_mute(a);
  spin(150);
  g_assert_cmpuint(zd_audio_status(a).percent, ==, 0);
  g_assert_false(zd_audio_status(a).muted);
  kill(pid, SIGTERM);
  waitpid(pid, NULL, 0);
  g_spawn_close_pid(pid);
  spin(200);
  g_assert_false(zd_audio_status(a).present);
  g_test_message("PASS child PID audio match, mute/unmute, 5 percent, 200 "
                 "percent cap, volume bubble timeout, stream cleanup");
  xfce_panel_plugin_provider_set_mode(XFCE_PANEL_PLUGIN_PROVIDER(p),
                                      XFCE_PANEL_PLUGIN_MODE_VERTICAL);
  spin(100);
  g_assert_cmpint(d->orientation, ==, GTK_ORIENTATION_VERTICAL);
  zd_minimize_all(d);
  spin(150);
  g_assert_true(xfw_window_is_minimized(a->window));
  g_assert_true(xfw_window_is_minimized(b->window));
  g_assert_nonnull(a->thumbnail);
  g_assert_nonnull(b->thumbnail);
  g_signal_emit_by_name(d->show_desktop_item, "activate");
  spin(150);
  g_assert_true(xfw_screen_get_show_desktop(d->screen));
  g_assert_true(gtk_check_menu_item_get_active(
      GTK_CHECK_MENU_ITEM(d->show_desktop_item)));
  g_signal_emit_by_name(d->show_desktop_item, "activate");
  spin(150);
  g_test_message("PASS vertical panel, minimize-all cache, show-desktop state");
  zd_save(d);
  zd_preview_schedule(a);
  spin(400);
  gtk_widget_destroy(f1);
  spin(200);
  g_assert_null(d->hover_button);
  g_assert_false(d->autohide_blocked);
  g_assert_cmpuint(g_hash_table_size(d->windows), ==, 1);
  gtk_widget_destroy(h);
  spin(100);
  gtk_widget_destroy(h2);
  spin(100);
  GtkWidget *h3;
  XfcePanelPlugin *p3 = host(rc, 9003, &h3);
  ZdDock *d3 = zd_get_dock(p3);
  spin(200);
  g_assert_true(((ZdButton *)d3->buttons->data)->pinned);
  g_assert_cmpstr(((ZdButton *)d3->buttons->data)->desktop, ==, desktop);
  zd_unpin(d3->buttons->data);
  g_assert_cmpuint(g_list_length(d3->buttons), ==, 1);
  gtk_widget_destroy(h3);
  gtk_widget_destroy(f2);
  spin(150);
  g_test_message("PASS closing hovered window safely, instance disposal, "
                 "reload pins, unpin");
  g_object_unref(app);
  g_remove(rc);
  g_remove(rc2);
  g_remove(desktop);
  g_remove(marker);
  g_rmdir(dir);
  g_free(rc);
  g_free(rc2);
  g_free(desktop);
  g_free(marker);
  g_free(dir);
}
static void pinned_lifecycle(void) {
  gchar *dir = g_dir_make_tmp("zero-dock-pins-XXXXXX", NULL);
  gchar *rc = g_build_filename(dir, "pins.rc", NULL),
        *desktop = g_build_filename(dir, "fixture.desktop", NULL),
        *other = g_build_filename(dir, "other.desktop", NULL),
        *binary = g_build_filename(g_getenv("ZERO_DOCK_TEST_BUILD"),
                                   "zero-dock-integration", NULL);
  gchar *entry =
      g_strdup_printf("[Desktop Entry]\nType=Application\nName=Launch fixture\n"
                      "Exec=\"%s\" --launch-fixture\nIcon=utilities-terminal\n"
                      "StartupWMClass=ZeroDockFixture\n",
                      binary);
  g_assert_true(g_file_set_contents(desktop, entry, -1, NULL));
  g_assert_true(
      g_file_set_contents(other,
                          "[Desktop Entry]\nType=Application\nName=Other app\n"
                          "Exec=/usr/bin/true\nStartupWMClass=OtherFixture\n",
                          -1, NULL));
  GKeyFile *config = g_key_file_new();
  const gchar *pins[] = {desktop, desktop, other};
  g_key_file_set_string_list(config, "Dock", "Pinned", pins, 3);
  g_assert_true(g_key_file_save_to_file(config, rc, NULL));
  g_key_file_unref(config);
  GtkWidget *h;
  XfcePanelPlugin *p = host(rc, 9010, &h);
  ZdDock *d = zd_get_dock(p);
  spin(200);
  g_assert_cmpuint(g_list_length(d->buttons), ==, 2);
  ZdButton *pin = d->buttons->data, *other_pin = d->buttons->next->data;
  GtkWidget *original_widget = pin->widget;
  gchar *original_key = g_strdup(pin->key);
  g_assert_null(pin->window);
  zd_preview_schedule(pin);
  g_assert_null(d->hover_button);

  /* Launch a separate process through the actual desktop entry. */
  gtk_button_clicked(GTK_BUTTON(pin->main));
  for (guint i = 0; i < 20 && !pin->window; i++)
    spin(100);
  g_assert_nonnull(pin->window);
  g_assert_true(pin->widget == original_widget);
  g_assert_true(d->buttons->data == pin);
  g_assert_cmpstr(pin->key, ==, original_key);
  g_assert_cmpuint(g_list_length(d->buttons), ==, 2);
  g_assert_cmpuint(g_hash_table_size(d->windows), ==, 1);
  g_assert_cmpuint(pin->number, ==, 0);
  g_assert_true(g_hash_table_lookup(d->windows, pin->window) == pin);
  zd_activate(pin);
  spin(200);
  gtk_button_clicked(GTK_BUTTON(pin->main));
  spin(200);
  g_assert_true(xfw_window_is_minimized(pin->window));
  gtk_button_clicked(GTK_BUTTON(pin->main));
  spin(200);
  g_assert_false(xfw_window_is_minimized(pin->window));
  g_assert_cmpuint(g_hash_table_size(d->windows), ==, 1);
  g_test_message("PASS fixed icon launches, reuses its widget and toggles the "
                 "sole window without creating another icon or process");

  mouse_at(pin->main);
  spin(50);
  Display *x = GDK_DISPLAY_XDISPLAY(gdk_display_get_default());
  XTestFakeButtonEvent(x, 2, True, CurrentTime);
  XTestFakeButtonEvent(x, 2, False, CurrentTime);
  XFlush(x);
  for (guint i = 0; i < 20 && g_hash_table_size(d->windows) < 2; i++)
    spin(100);
  g_assert_cmpuint(g_hash_table_size(d->windows), ==, 2);
  g_assert_cmpuint(g_list_length(d->buttons), ==, 3);
  ZdButton *second = g_list_last(d->buttons)->data;
  g_assert_false(second->pinned);
  g_assert_nonnull(second->window);
  g_assert_cmpuint(pin->number, >, 0);
  g_assert_cmpuint(second->number, >, 0);
  g_assert_cmpuint(pin->number, !=, second->number);
  zd_minimize(second);
  spin(200);
  GdkPixbuf *frame = second->thumbnail ? g_object_ref(second->thumbnail) : NULL;
  g_assert_nonnull(frame);
  XfwWindow *remaining = g_object_ref(second->window);
  zd_preview_schedule(pin);
  spin(400);
  g_assert_true(d->autohide_blocked);
  xfw_window_close(pin->window, zd_timestamp(d), NULL);
  spin(500);
  g_assert_true(pin->window == remaining);
  g_assert_true(pin->thumbnail == frame);
  g_assert_true(pin->widget == original_widget);
  g_assert_true(d->buttons->data == pin);
  g_assert_true(d->buttons->next->data == other_pin);
  g_assert_cmpuint(g_list_length(d->buttons), ==, 2);
  g_assert_cmpuint(g_hash_table_size(d->windows), ==, 1);
  g_assert_cmpuint(pin->number, ==, 0);
  g_assert_null(d->hover_button);
  g_assert_false(d->autohide_blocked);
  g_object_unref(frame);
  g_test_message("PASS middle click opens a separate second window; closing "
                 "the first transfers the remaining window and cached frame "
                 "to the fixed position and releases the preview");

  zd_unpin(pin);
  g_assert_false(pin->pinned);
  g_assert_null(pin->desktop);
  g_assert_true(pin->window == remaining);
  g_assert_true(g_str_has_prefix(pin->key, "window:"));
  g_assert_cmpuint(g_hash_table_size(d->windows), ==, 1);
  g_assert_cmpuint(g_list_length(d->buttons), ==, 2);
  GDesktopAppInfo *app = g_desktop_app_info_new_from_filename(desktop);
  zd_pin_app(d, app);
  g_object_unref(app);
  g_assert_true(pin->pinned);
  g_assert_cmpstr(pin->key, ==, original_key);
  xfw_window_close(pin->window, zd_timestamp(d), NULL);
  spin(500);
  g_assert_null(pin->window);
  g_assert_null(pin->thumbnail);
  g_assert_cmpuint(g_hash_table_size(d->windows), ==, 0);
  g_assert_cmpuint(g_list_length(d->buttons), ==, 2);
  g_assert_true(gtk_widget_get_visible(pin->widget));
  g_assert_false(gtk_widget_get_visible(pin->sound));
  g_assert_nonnull(gtk_widget_get_tooltip_text(pin->main));
  zd_preview_schedule(pin);
  g_assert_null(d->hover_button);
  g_object_unref(remaining);

  /* Class metadata may arrive late or change after a splash window. */
  GtkWidget *w = fixture("Changing application identity");
  spin(200);
  g_assert_true(button(d, w) == pin);
  XClassHint hint = {"OtherFixture", "OtherFixture"};
  XSetClassHint(x, GDK_WINDOW_XID(gtk_widget_get_window(w)), &hint);
  XFlush(x);
  spin(200);
  g_assert_null(pin->window);
  g_assert_true(button(d, w) == other_pin);
  hint.res_name = hint.res_class = "UnknownZeroDockIdentity";
  XSetClassHint(x, GDK_WINDOW_XID(gtk_widget_get_window(w)), &hint);
  XFlush(x);
  spin(200);
  g_assert_null(other_pin->window);
  g_assert_false(button(d, w)->pinned);
  g_assert_cmpuint(g_list_length(d->buttons), ==, 3);
  hint.res_name = hint.res_class = "ZeroDockFixture";
  XSetClassHint(x, GDK_WINDOW_XID(gtk_widget_get_window(w)), &hint);
  XFlush(x);
  spin(200);
  g_assert_true(button(d, w) == pin);
  g_assert_cmpuint(g_list_length(d->buttons), ==, 2);
  g_assert_cmpuint(g_hash_table_size(d->windows), ==, 1);
  gtk_widget_destroy(w);
  spin(200);
  g_assert_null(pin->window);
  g_test_message("PASS changing or late window classes rebind the correct "
                 "fixed app, while unmatched windows remain independent");

  zd_move_button(d, pin, other_pin, TRUE);
  gtk_widget_destroy(h);
  spin(100);
  p = host(rc, 9011, &h);
  d = zd_get_dock(p);
  spin(100);
  g_assert_cmpuint(g_list_length(d->buttons), ==, 2);
  g_assert_cmpstr(((ZdButton *)d->buttons->data)->desktop, ==, other);
  g_assert_cmpstr(((ZdButton *)d->buttons->next->data)->desktop, ==, desktop);
  zd_unpin(d->buttons->next->data);
  g_assert_cmpuint(g_list_length(d->buttons), ==, 1);
  gtk_widget_destroy(h);
  spin(100);
  g_test_message(
      "PASS unpinning keeps the live window, the last close restores "
      "the launcher, duplicate saved pins are removed, and fixed "
      "order survives reload");
  g_remove(rc);
  g_remove(desktop);
  g_remove(other);
  g_rmdir(dir);
  g_free(dir);
  g_free(rc);
  g_free(desktop);
  g_free(other);
  g_free(binary);
  g_free(entry);
  g_free(original_key);
}
static GtkWidget *find_preference(GtkWidget *widget, ZdDock *d) {
  GtkWidget *reset = NULL;
  if (GTK_IS_SPIN_BUTTON(widget)) {
    guint *field = g_object_get_data(G_OBJECT(widget), "setting");
    if (field == &d->preview_width)
      gtk_spin_button_set_value(GTK_SPIN_BUTTON(widget), 420);
    else if (field == &d->preview_delay)
      gtk_spin_button_set_value(GTK_SPIN_BUTTON(widget), 150);
    else if (field == &d->preview_interval)
      gtk_spin_button_set_value(GTK_SPIN_BUTTON(widget), 250);
  } else if (GTK_IS_BUTTON(widget) &&
             !g_strcmp0(gtk_button_get_label(GTK_BUTTON(widget)),
                        _("恢复默认设置（保留固定应用）")))
    reset = widget;
  if (GTK_IS_CONTAINER(widget)) {
    GList *children = gtk_container_get_children(GTK_CONTAINER(widget));
    for (GList *l = children; l; l = l->next) {
      GtkWidget *candidate = find_preference(l->data, d);
      if (candidate)
        reset = candidate;
    }
    g_list_free(children);
  }
  return reset;
}

static void launch_feedback(void) {
  gchar *dir = g_dir_make_tmp("zero-dock-launch-XXXXXX", NULL);
  gchar *rc = g_build_filename(dir, "settings.rc", NULL),
        *desktop = g_build_filename(dir, "slow.desktop", NULL),
        *timeout = g_build_filename(dir, "no-window.desktop", NULL),
        *failure = g_build_filename(dir, "failure.desktop", NULL),
        *missing = g_build_filename(dir, "missing.desktop", NULL),
        *binary = g_build_filename(g_getenv("ZERO_DOCK_TEST_BUILD"),
                                   "zero-dock-integration", NULL);
  gchar *entry = g_strdup_printf(
      "[Desktop Entry]\nType=Application\nName=Slow fixture\n"
      "Exec=\"%s\" --launch-fixture=500\nStartupWMClass=ZeroDockFixture\n",
      binary);
  g_assert_true(g_file_set_contents(desktop, entry, -1, NULL));
  g_assert_true(g_file_set_contents(
      timeout,
      "[Desktop Entry]\nType=Application\nName=No window\nExec=/usr/bin/true\n",
      -1, NULL));
  gchar *bad = g_strdup_printf("[Desktop "
                               "Entry]\nType=Application\nName=Failure\nExec=/"
                               "usr/bin/true\nPath=%s/missing-directory\n",
                               dir);
  g_assert_true(g_file_set_contents(failure, bad, -1, NULL));
  GtkWidget *h;
  XfcePanelPlugin *p = host(rc, 9020, &h);
  ZdDock *d = zd_get_dock(p);
  ZdButton *pin = zd_add_pin(d, desktop), *no_window = zd_add_pin(d, timeout),
           *fail = zd_add_pin(d, failure), *lost = zd_add_pin(d, missing);
  g_assert_nonnull(fail->app);
  g_assert_nonnull(lost);
  g_assert_null(lost->app);
  zd_refresh(d);
  gtk_button_clicked(GTK_BUTTON(pin->main));
  pid_t pid = pin->launch_pid;
  g_assert_cmpint(pid, >, 1);
  g_assert_true(pin->launching);
  gtk_button_clicked(GTK_BUTTON(pin->main));
  gtk_button_clicked(GTK_BUTTON(pin->main));
  g_assert_cmpint(pin->launch_pid, ==, pid);
  g_assert_null(pin->window);
  spin(900);
  g_assert_nonnull(pin->window);
  g_assert_false(pin->launching);
  g_assert_null(pin->launch_error);
  g_assert_cmpuint(g_hash_table_size(d->windows), ==, 1);
  g_assert_cmpint(zd_window_pid(pin->window), ==, pid);
  g_test_message("PASS repeated left clicks during slow startup submit one "
                 "process and clear feedback when its window arrives");
  GdkPixbuf *cached = g_object_ref(pin->icon);
  gint64 begin = g_get_monotonic_time();
  for (guint i = 0; i < 100; i++)
    zd_update_audio_buttons(d);
  g_assert_true(cached == pin->icon);
  g_test_message("100 audio-only updates: %.2f ms; window icon retained",
                 (g_get_monotonic_time() - begin) / 1000.);
  GdkPixbuf *replacement = gdk_pixbuf_new(GDK_COLORSPACE_RGB, TRUE, 8, 32, 32);
  gdk_pixbuf_fill(replacement, 0x3399ffff);
  /* Exercise actual icon invalidation on a local fixture. */
  GtkWidget *w = fixture("Icon change fixture");
  spin(200);
  ZdButton *running = button(d, w);
  GdkPixbuf *old = g_object_ref(running->icon);
  gtk_window_set_icon(GTK_WINDOW(w), replacement);
  spin(200);
  g_assert_true(running->icon != old);
  g_object_unref(old);
  g_object_unref(replacement);
  g_object_unref(cached);
  gtk_widget_destroy(w);
  spin(100);
  d->launch_timeout = 200;
  gtk_button_clicked(GTK_BUTTON(no_window->main));
  g_assert_true(no_window->launching);
  spin(350);
  g_assert_false(no_window->launching);
  g_assert_nonnull(no_window->launch_error);
  gtk_button_clicked(GTK_BUTTON(no_window->main));
  g_assert_true(no_window->launching);
  zd_launch(fail);
  g_assert_false(fail->launching);
  g_assert_nonnull(fail->launch_error);
  g_assert_nonnull(d->error_dialog);
  gtk_widget_destroy(d->error_dialog);
  zd_launch(lost);
  g_assert_nonnull(d->error_dialog);
  gtk_widget_destroy(d->error_dialog);
  zd_save(d);
  GKeyFile *config = g_key_file_new();
  g_assert_true(g_key_file_load_from_file(config, rc, G_KEY_FILE_NONE, NULL));
  gsize count;
  gchar **saved =
      g_key_file_get_string_list(config, "Dock", "Pinned", &count, NULL);
  g_assert_cmpuint(count, ==, 4);
  g_assert_cmpstr(saved[3], ==, missing);
  g_strfreev(saved);
  g_key_file_unref(config);
  gchar *diagnostic = zd_diagnostics(d);
  g_assert_nonnull(strstr(diagnostic, ZERO_DOCK_VERSION));
  g_assert_null(strstr(diagnostic, dir));
  g_assert_null(strstr(diagnostic, "Slow fixture"));
  g_free(diagnostic);
  zd_configure(p, d);
  GtkWidget *content = gtk_dialog_get_content_area(GTK_DIALOG(d->settings));
  GtkWidget *reset = find_preference(content, d);
  g_assert_cmpuint(d->preview_width, ==, 420);
  g_assert_cmpuint(d->preview_delay, ==, 150);
  g_assert_cmpuint(d->preview_interval, ==, 250);
  config = g_key_file_new();
  g_assert_true(g_key_file_load_from_file(config, rc, G_KEY_FILE_NONE, NULL));
  g_assert_cmpint(g_key_file_get_integer(config, "Dock", "PreviewWidth", NULL),
                  ==, 420);
  g_key_file_unref(config);
  g_assert_nonnull(reset);
  gtk_button_clicked(GTK_BUTTON(reset));
  g_assert_cmpuint(d->preview_width, ==, 300);
  g_assert_cmpuint(d->launch_timeout, ==, 10000);
  g_assert_cmpuint(g_list_length(d->buttons), ==, 4);
  gtk_widget_destroy(d->settings);
  g_test_message("PASS native preference controls persist values and restoring "
                 "defaults preserves all fixed apps, including missing files");
  xfw_window_close(pin->window, zd_timestamp(d), NULL);
  spin(200);
  gtk_button_clicked(GTK_BUTTON(no_window->main));
  gtk_widget_destroy(h);
  spin(300);
  g_assert_true(
      g_file_set_contents(rc, "[Dock]\nPinned=original;\n[broken", -1, NULL));
  p = host(rc, 9021, &h);
  d = zd_get_dock(p);
  g_assert_nonnull(d->error_dialog);
  g_assert_false(d->save_blocked);
  GDir *directory = g_dir_open(dir, 0, NULL);
  const gchar *name;
  gchar *backup = NULL;
  while ((name = g_dir_read_name(directory)))
    if (g_str_has_prefix(name, "settings.rc.invalid-")) {
      backup = g_build_filename(dir, name, NULL);
      break;
    }
  g_dir_close(directory);
  g_assert_nonnull(backup);
  gchar *preserved = NULL;
  g_assert_true(g_file_get_contents(backup, &preserved, NULL, NULL));
  g_assert_cmpstr(preserved, ==, "[Dock]\nPinned=original;\n[broken");
  gtk_widget_destroy(h);
  spin(100);
  g_remove(backup);
  g_free(backup);
  g_free(preserved);
  g_test_message(
      "PASS launch timeout/retry, visible spawn errors, missing "
      "launcher persistence, cached icons, minimal diagnostics "
      "and disposal during startup; malformed configs are backed up");
  g_remove(desktop);
  g_remove(timeout);
  g_remove(failure);
  g_remove(rc);
  g_rmdir(dir);
  g_free(dir);
  g_free(rc);
  g_free(desktop);
  g_free(timeout);
  g_free(failure);
  g_free(missing);
  g_free(binary);
  g_free(entry);
  g_free(bad);
}
static void identity_workspaces(void) {
  gchar *dir = g_dir_make_tmp("zero-dock-identity-XXXXXX", NULL);
  gchar *rc = g_build_filename(dir, "settings.rc", NULL),
        *first = g_build_filename(dir, "first.desktop", NULL),
        *second = g_build_filename(dir, "second.desktop", NULL);
  const gchar *entry =
      "[Desktop Entry]\nType=Application\nName=Identity fixture\n"
      "Exec=/usr/bin/true\nStartupWMClass=ZeroDockFixture\n";
  g_assert_true(g_file_set_contents(first, entry, -1, NULL));
  g_assert_true(g_file_set_contents(second, entry, -1, NULL));
  GtkWidget *h;
  XfcePanelPlugin *p = host(rc, 9030, &h);
  gtk_window_stick(GTK_WINDOW(h));
  ZdDock *d = zd_get_dock(p);
  ZdButton *a = zd_add_pin(d, first), *b = zd_add_pin(d, second);
  GtkWidget *w = fixture("Identity fixture");
  spin(200);
  g_assert_true(button(d, w) == a);
  Display *x = GDK_DISPLAY_XDISPLAY(gdk_display_get_default());
  Window xid = GDK_WINDOW_XID(gtk_widget_get_window(w));
  Atom gtk_id = XInternAtom(x, "_GTK_APPLICATION_ID", False),
       desktop_id = XInternAtom(x, "_KDE_NET_WM_DESKTOP_FILE", False),
       utf8 = XInternAtom(x, "UTF8_STRING", False);
  XChangeProperty(x, xid, gtk_id, utf8, 8, PropModeReplace,
                  (const unsigned char *)"second", 6);
  XFlush(x);
  spin(200);
  g_assert_true(button(d, w) == b);
  g_assert_null(a->window);
  XDeleteProperty(x, xid, gtk_id);
  XChangeProperty(x, xid, desktop_id, utf8, 8, PropModeReplace,
                  (const unsigned char *)first, strlen(first));
  XFlush(x);
  spin(200);
  g_assert_true(button(d, w) == a);
  g_assert_cmpuint(g_list_length(d->buttons), ==, 2);
  g_assert_cmpuint(g_hash_table_size(d->windows), ==, 1);
  zd_unpin(b);
  GtkWidget *w2 = fixture("Workspace fixture");
  spin(200);
  g_assert_cmpuint(g_hash_table_size(d->windows), ==, 2);
  XfwWorkspaceManager *manager = xfw_screen_get_workspace_manager(d->screen);
  GList *spaces = xfw_workspace_manager_list_workspaces(manager);
  g_assert_cmpuint(g_list_length(spaces), >=, 2);
  XfwWorkspace *original = xfw_window_get_workspace(a->window), *other = NULL;
  for (GList *l = spaces; l; l = l->next)
    if (l->data != original) {
      other = l->data;
      break;
    }
  g_assert_nonnull(other);
  d->all_workspaces = FALSE;
  xfw_window_move_to_workspace(a->window, other, NULL);
  spin(200);
  g_assert_true(button(d, w2) == a);
  g_assert_false(gtk_widget_get_visible(button(d, w)->widget));
  g_assert_true(gtk_widget_get_visible(a->widget));
  g_assert_cmpuint(a->number, ==, 0);
  xfw_workspace_activate(other, NULL);
  spin(200);
  g_assert_true(button(d, w) == a);
  g_assert_false(gtk_widget_get_visible(button(d, w2)->widget));
  g_assert_cmpuint(g_list_length(d->buttons), ==, 2);
  g_assert_cmpuint(g_hash_table_size(d->windows), ==, 2);
  GdkEvent *event = gdk_event_new(GDK_BUTTON_PRESS);
  event->button.window = g_object_ref(gtk_widget_get_window(a->main));
  event->button.button = 3;
  event->button.time = zd_timestamp(d);
  gdk_event_set_device(event, gdk_seat_get_pointer(gdk_display_get_default_seat(
                                  gdk_display_get_default())));
  zd_window_menu(a, event);
  GList *items = gtk_container_get_children(GTK_CONTAINER(d->menu));
  GtkWidget *list = gtk_menu_item_get_submenu(GTK_MENU_ITEM(items->data));
  g_assert_nonnull(list);
  GList *windows = gtk_container_get_children(GTK_CONTAINER(list));
  g_assert_cmpuint(g_list_length(windows), ==, 2);
  for (GList *l = windows; l; l = l->next) {
    XfwWindow *target = g_object_get_data(G_OBJECT(l->data), "window");
    if (target == button(d, w2)->window) {
      g_signal_emit_by_name(l->data, "activate");
      break;
    }
  }
  g_list_free(windows);
  g_list_free(items);
  gdk_event_free(event);
  spin(200);
  g_assert_true(xfw_workspace_get_state(original) & XFW_WORKSPACE_STATE_ACTIVE);
  g_assert_true(button(d, w2) == a);
  if (d->menu)
    gtk_widget_destroy(d->menu);
  gtk_widget_destroy(w);
  gtk_widget_destroy(w2);
  spin(200);
  g_assert_null(a->window);
  g_assert_true(gtk_widget_get_visible(a->widget));
  gtk_widget_destroy(h);
  spin(100);
  g_test_message("PASS late GTK/desktop application IDs distinguish shared "
                 "window classes; workspace changes reuse the current window "
                 "without duplicates and the app menu selects other windows");
  g_remove(first);
  g_remove(second);
  g_remove(rc);
  g_rmdir(dir);
  g_free(dir);
  g_free(rc);
  g_free(first);
  g_free(second);
}
static void improvements(void) {
  gchar *dir = g_dir_make_tmp("zero-dock-improve-XXXXXX", NULL);
  gchar *rc = g_build_filename(dir, "settings.rc", NULL),
        *desktop = g_build_filename(dir, "manual.desktop", NULL),
        *backup = g_build_filename(dir, "backup.rc", NULL),
        *bad = g_build_filename(dir, "bad.rc", NULL);
  g_assert_true(g_file_set_contents(
      desktop,
      "[Desktop Entry]\nType=Application\nName=Manual "
      "association\nExec=/usr/bin/true\nStartupWMClass=DistinctManual\n",
      -1, NULL));
  GtkWidget *h;
  XfcePanelPlugin *p = host(rc, 9040, &h);
  ZdDock *d = zd_get_dock(p);
  ZdButton *pin = zd_add_pin(d, desktop);
  GtkWidget *first = fixture("Manual association fixture");
  spin(200);
  ZdButton *running = button(d, first);
  XfwWindow *window = g_object_ref(running->window);
  GError *error = NULL;
  g_assert_false(zd_associate(d, window, "/missing.desktop", &error));
  g_assert_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT);
  g_clear_error(&error);
  g_assert_cmpuint(g_hash_table_size(d->associations), ==, 0);
  g_assert_true(zd_associate(d, window, desktop, &error));
  spin(150);
  g_assert_true(button(d, first) == pin);
  g_assert_true(gtk_widget_get_can_focus(pin->main));
  g_assert_cmpuint(g_hash_table_size(d->windows), ==, 1);
  d->max_visible = 3;
  d->left_action = 1;
  d->middle_action = 2;
  d->scroll_windows = FALSE;
  GtkWidget *others[7];
  for (guint i = 0; i < G_N_ELEMENTS(others); i++)
    others[i] = fixture("Overflow fixture");
  spin(400);
  guint visible = 0;
  for (GList *l = d->buttons; l; l = l->next)
    visible += gtk_widget_get_visible(((ZdButton *)l->data)->widget);
  g_assert_cmpuint(visible, ==, 2);
  g_assert_true(gtk_widget_get_visible(d->overflow));
  g_assert_cmpuint(g_hash_table_size(d->windows), ==, 8);
  zd_overflow_menu(d, NULL);
  GList *items = gtk_container_get_children(GTK_CONTAINER(d->menu));
  g_assert_cmpuint(g_list_length(items), ==, 6);
  ZdButton *last = button(d, others[6]);
  const gchar *target_key = g_strdup(last->key);
  for (GList *l = items; l; l = l->next)
    if (!g_strcmp0(g_object_get_data(G_OBJECT(l->data), "button-key"),
                   target_key)) {
      g_signal_emit_by_name(l->data, "activate");
      break;
    }
  g_free((gpointer)target_key);
  g_list_free(items);
  if (d->menu)
    gtk_widget_destroy(d->menu);
  spin(100);
  g_assert_true(xfw_window_is_active(last->window));
  gtk_button_clicked(GTK_BUTTON(last->main));
  spin(100);
  g_assert_false(xfw_window_is_minimized(last->window));
  gtk_widget_grab_focus(pin->main);
  GdkEventKey key = {.type = GDK_KEY_PRESS, .keyval = GDK_KEY_End};
  gboolean handled = FALSE;
  g_signal_emit_by_name(pin->main, "key-press-event", &key, &handled);
  g_assert_true(handled);
  g_assert_true(gtk_window_get_focus(GTK_WINDOW(h)) == d->overflow);
  key.keyval = GDK_KEY_Home;
  g_signal_emit_by_name(d->overflow, "key-press-event", &key, &handled);
  g_assert_true(gtk_window_get_focus(GTK_WINDOW(h)) == pin->main);
  g_assert_true(zd_config_export(d, backup, &error));
  zd_association_clear(pin);
  d->max_visible = 0;
  zd_settings_defaults(d);
  g_assert_true(zd_config_import(d, backup, &error));
  spin(150);
  g_assert_cmpuint(d->max_visible, ==, 3);
  g_assert_cmpuint(d->left_action, ==, 1);
  g_assert_cmpuint(d->middle_action, ==, 2);
  g_assert_false(d->scroll_windows);
  g_assert_cmpuint(g_hash_table_size(d->associations), ==, 1);
  g_assert_cmpuint(g_hash_table_size(d->windows), ==, 8);
  pin = button(d, first);
  g_assert_true(pin->pinned);
  gchar *before = NULL, *after = NULL;
  g_assert_true(g_file_get_contents(rc, &before, NULL, NULL));
  g_assert_true(g_file_set_contents(bad, "[Other]\nKey=bad\n", -1, NULL));
  g_assert_false(zd_config_import(d, bad, &error));
  g_assert_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA);
  g_clear_error(&error);
  g_assert_true(g_file_get_contents(rc, &after, NULL, NULL));
  g_assert_cmpstr(before, ==, after);
  g_free(before);
  g_free(after);
  d->max_visible = 1;
  zd_update_buttons(d);
  g_assert_false(gtk_widget_get_visible(pin->widget));
  g_assert_true(gtk_widget_get_visible(d->overflow));
  d->max_visible = 0;
  zd_update_buttons(d);
  zd_preview_schedule(pin);
  spin(500);
  g_assert_true(gtk_widget_get_visible(d->preview));
  g_assert_nonnull(d->preview_close);
  GtkSettings *theme = gtk_settings_get_default();
  g_object_set(theme, "gtk-theme-name", "Adwaita",
               "gtk-application-prefer-dark-theme", FALSE, NULL);
  spin(150);
  evidence(d->preview, "0.3-light-preview.png");
  GdkRGBA light, dark;
  g_assert_true(gtk_style_context_lookup_color(
      gtk_widget_get_style_context(d->preview), "theme_bg_color", &light));
  g_object_set(theme, "gtk-application-prefer-dark-theme", TRUE, NULL);
  spin(150);
  evidence(d->preview, "0.3-dark-preview.png");
  g_assert_true(gtk_style_context_lookup_color(
      gtk_widget_get_style_context(d->preview), "theme_bg_color", &dark));
  g_assert_cmpfloat(light.red, >, dark.red);
  g_assert_cmpstr(gtk_label_get_text(GTK_LABEL(d->preview_title)), ==,
                  "Manual association fixture");
  g_assert_cmpstr(gtk_label_get_text(GTK_LABEL(d->preview_workspace)), !=, "");
  gint x, y, width, height;
  gtk_window_get_position(GTK_WINDOW(d->preview), &x, &y);
  gtk_window_get_size(GTK_WINDOW(d->preview), &width, &height);
  GdkRectangle bounds;
  GdkMonitor *monitor = gdk_display_get_monitor_at_window(
      gdk_display_get_default(), gtk_widget_get_window(h));
  gdk_monitor_get_workarea(monitor, &bounds);
  g_assert_cmpint(x, >=, bounds.x);
  g_assert_cmpint(y, >=, bounds.y);
  g_assert_cmpint(x + width, <=, bounds.x + bounds.width);
  g_assert_cmpint(y + height, <=, bounds.y + bounds.height);
  g_signal_emit_by_name(d->screen, "monitors-changed");
  g_assert_null(d->hover_button);
  g_assert_false(d->autohide_blocked);
  xfce_panel_plugin_provider_set_mode(XFCE_PANEL_PLUGIN_PROVIDER(p),
                                      XFCE_PANEL_PLUGIN_MODE_VERTICAL);
  zd_preview_schedule(pin);
  spin(500);
  gtk_window_get_position(GTK_WINDOW(d->preview), &x, &y);
  gtk_window_get_size(GTK_WINDOW(d->preview), &width, &height);
  g_assert_cmpint(x, >=, bounds.x);
  g_assert_cmpint(y, >=, bounds.y);
  g_assert_cmpint(x + width, <=, bounds.x + bounds.width);
  g_assert_cmpint(y + height, <=, bounds.y + bounds.height);
  for (GList *l = d->buttons; l; l = l->next) {
    ZdButton *b = l->data;
    g_clear_object(&b->thumbnail);
    b->thumbnail = gdk_pixbuf_new(GDK_COLORSPACE_RGB, TRUE, 8, 2048, 1024);
  }
  zd_trim_frames(d, pin);
  gsize total = 0;
  for (GList *l = d->buttons; l; l = l->next) {
    ZdButton *b = l->data;
    if (b->thumbnail)
      total += gdk_pixbuf_get_byte_length(b->thumbnail);
  }
  g_assert_cmpuint(total, <=, 32 * 1024 * 1024);
  g_assert_nonnull(pin->thumbnail);
  XfwWindow *closing = g_object_ref(pin->window);
  gtk_button_clicked(GTK_BUTTON(d->preview_close));
  spin(200);
  g_assert_false(g_hash_table_contains(d->windows, closing));
  g_object_unref(closing);
  g_assert_cmpuint(g_hash_table_size(d->windows), ==, 7);
  g_assert_nonnull(pin->window);
  g_assert_false(d->autohide_blocked);
  g_object_unref(window);
  for (guint i = 0; i < G_N_ELEMENTS(others); i++)
    gtk_widget_destroy(others[i]);
  spin(200);
  gtk_widget_destroy(h);
  spin(100);
  p = host(rc, 9041, &h);
  d = zd_get_dock(p);
  first = fixture("Rule restored after reload");
  spin(200);
  g_assert_true(button(d, first)->pinned);
  g_assert_cmpuint(g_hash_table_size(d->associations), ==, 1);
  g_test_message(
      "PASS manual rules survive reload; overflow selection, focus navigation, "
      "click settings, safe configuration restore, monitor invalidation, both "
      "popup orientations and 32 MiB frame cache budget (scale %d)",
      gtk_widget_get_scale_factor(d->box));
  gtk_widget_destroy(first);
  gtk_widget_destroy(h);
  spin(100);
  GDir *directory = g_dir_open(dir, 0, NULL);
  const gchar *name;
  while ((name = g_dir_read_name(directory))) {
    gchar *file = g_build_filename(dir, name, NULL);
    g_remove(file);
    g_free(file);
  }
  g_dir_close(directory);
  g_rmdir(dir);
  g_free(dir);
  g_free(rc);
  g_free(desktop);
  g_free(backup);
  g_free(bad);
}

static guint resident_kib(void) {
  gchar *text = NULL;
  guint rss = 0;
  if (g_file_get_contents("/proc/self/status", &text, NULL, NULL)) {
    gchar *line = strstr(text, "VmRSS:");
    if (line)
      sscanf(line, "VmRSS: %u", &rss);
  }
  g_free(text);
  return rss;
}
static void stress(void) {
  guint seconds =
      g_getenv("ZERO_DOCK_SOAK_SECONDS")
          ? g_ascii_strtoull(g_getenv("ZERO_DOCK_SOAK_SECONDS"), NULL, 10)
          : 0;
  if (!seconds) {
    g_test_skip("Enable with --soak-seconds; use 86400 for a 24-hour run");
    return;
  }
  gchar *dir = g_dir_make_tmp("zero-dock-stress-XXXXXX", NULL),
        *rc = g_build_filename(dir, "settings.rc", NULL);
  GtkWidget *h;
  XfcePanelPlugin *p = host(rc, 9050, &h);
  ZdDock *d = zd_get_dock(p);
  guint initial = resident_kib(), warm = 0, peak = initial, cycles = 0;
  gint64 start = g_get_monotonic_time(),
         end = start + (gint64)seconds * G_USEC_PER_SEC;
  do {
    GtkWidget *windows[32];
    for (guint i = 0; i < G_N_ELEMENTS(windows); i++)
      windows[i] = fixture("Stress fixture");
    spin(250);
    g_assert_cmpuint(g_hash_table_size(d->windows), ==, 32);
    g_assert_true(gtk_widget_get_visible(d->overflow));
    for (guint i = 0; i < 8; i++) {
      GdkPixbuf *frame = zd_capture(button(d, windows[i]));
      g_clear_object(&frame);
    }
    for (guint i = 0; i < 30; i++) {
      zd_update_audio_buttons(d);
      zd_update_buttons(d);
    }
    peak = MAX(peak, resident_kib());
    for (guint i = 0; i < G_N_ELEMENTS(windows); i++)
      gtk_widget_destroy(windows[i]);
    spin(150);
    g_assert_cmpuint(g_hash_table_size(d->windows), ==, 0);
    g_assert_null(d->buttons);
    g_assert_null(d->hover_button);
    g_assert_false(d->autohide_blocked);
    cycles++;
    if (cycles == 5)
      warm = resident_kib();
  } while (g_get_monotonic_time() < end);
  guint final = resident_kib();
  struct rusage cpu_before, cpu_after;
  getrusage(RUSAGE_SELF, &cpu_before);
  gint64 idle_start = g_get_monotonic_time();
  spin(10000);
  getrusage(RUSAGE_SELF, &cpu_after);
  double cpu_us = (cpu_after.ru_utime.tv_sec - cpu_before.ru_utime.tv_sec +
                   cpu_after.ru_stime.tv_sec - cpu_before.ru_stime.tv_sec) *
                      1000000. +
                  cpu_after.ru_utime.tv_usec - cpu_before.ru_utime.tv_usec +
                  cpu_after.ru_stime.tv_usec - cpu_before.ru_stime.tv_usec;
  g_test_message("Idle CPU after stress: %.3f%% over 10 s",
                 100. * cpu_us / (g_get_monotonic_time() - idle_start));
  g_assert_cmpuint(final, <, MAX(warm, initial) + 128 * 1024);
  g_test_message("Lifecycle stress: %.1f s, %u cycles, %u created/closed "
                 "windows; RSS initial=%u warm=%u final=%u peak=%u KiB",
                 (g_get_monotonic_time() - start) / 1000000., cycles,
                 cycles * 32, initial, warm, final, peak);
  gtk_widget_destroy(h);
  spin(100);
  g_remove(rc);
  g_rmdir(dir);
  g_free(rc);
  g_free(dir);
}
static void audio_recovery(void) {
  const gchar *server = g_getenv("ZERO_DOCK_PRIVATE_PULSE_PID");
  if (!server) {
    g_test_skip("Requires --private-audio; never disconnect the user's server");
    return;
  }
  GPid pulse_pid = g_ascii_strtoull(server, NULL, 10);
  gchar *proc = g_strdup_printf("/proc/%d/cmdline", pulse_pid), *cmdline = NULL;
  g_assert_true(g_file_get_contents(proc, &cmdline, NULL, NULL));
  g_assert_nonnull(strstr(cmdline, "pipewire-pulse"));
  g_free(proc);
  g_free(cmdline);
  gchar *dir = g_dir_make_tmp("zero-dock-recovery-XXXXXX", NULL),
        *rc = g_build_filename(dir, "settings.rc", NULL);
  GtkWidget *h, *w = fixture("Private audio recovery fixture");
  XfcePanelPlugin *p = host(rc, 9060, &h);
  ZdDock *d = zd_get_dock(p);
  spin(200);
  ZdButton *b = button(d, w);
  gchar *stream_args[] = {"/usr/bin/pacat", "--playback",   "--raw",
                          "--rate=8000",    "--channels=1", "--format=s16le",
                          "--volume=0",     "/dev/zero",    NULL};
  GPid stream;
  g_assert_true(g_spawn_async(NULL, stream_args, NULL,
                              G_SPAWN_DO_NOT_REAP_CHILD, NULL, NULL, &stream,
                              NULL));
  spin(600);
  g_assert_true(zd_audio_status(b).present);
  for (guint cycle = 0; cycle < 2; cycle++) {
    kill(pulse_pid, SIGTERM);
    if (cycle) {
      waitpid(pulse_pid, NULL, 0);
      g_spawn_close_pid(pulse_pid);
    }
    spin(500);
    g_assert_false(zd_audio_status(b).present);
    g_assert_false(gtk_widget_get_visible(b->sound));
    kill(stream, SIGTERM);
    waitpid(stream, NULL, 0);
    g_spawn_close_pid(stream);
    gchar *pulse_args[] = {"/usr/bin/pipewire-pulse", NULL};
    g_assert_true(g_spawn_async(NULL, pulse_args, NULL,
                                G_SPAWN_DO_NOT_REAP_CHILD |
                                    G_SPAWN_STDOUT_TO_DEV_NULL |
                                    G_SPAWN_STDERR_TO_DEV_NULL,
                                NULL, NULL, &pulse_pid, NULL));
    spin(800);
    gchar *sink_args[] = {"/usr/bin/pactl", "load-module", "module-null-sink",
                          "sink_name=zero-dock-recovery", NULL};
    gint status;
    g_assert_true(
        g_spawn_sync(NULL, sink_args, NULL,
                     G_SPAWN_STDOUT_TO_DEV_NULL | G_SPAWN_STDERR_TO_DEV_NULL,
                     NULL, NULL, NULL, NULL, &status, NULL));
    g_assert_true(g_spawn_check_wait_status(status, NULL));
    gchar *default_args[] = {"/usr/bin/pactl", "set-default-sink",
                             "zero-dock-recovery", NULL};
    g_assert_true(g_spawn_sync(NULL, default_args, NULL,
                               G_SPAWN_STDOUT_TO_DEV_NULL, NULL, NULL, NULL,
                               NULL, &status, NULL));
    g_assert_true(g_spawn_check_wait_status(status, NULL));
    g_assert_true(g_spawn_async(NULL, stream_args, NULL,
                                G_SPAWN_DO_NOT_REAP_CHILD, NULL, NULL, &stream,
                                NULL));
    spin(4500);
    g_assert_true(zd_audio_status(b).present);
    gboolean muted = zd_audio_status(b).muted;
    zd_audio_mute(b);
    spin(150);
    g_assert_cmpint(zd_audio_status(b).muted, ==, !muted);
  }
  kill(stream, SIGTERM);
  waitpid(stream, NULL, 0);
  g_spawn_close_pid(stream);
  gtk_widget_destroy(w);
  gtk_widget_destroy(h);
  spin(100);
  kill(pulse_pid, SIGTERM);
  waitpid(pulse_pid, NULL, 0);
  g_spawn_close_pid(pulse_pid);
  g_test_message("PASS two private audio server shutdown/restart cycles clear "
                 "stale badges and restore stream controls");
  g_remove(rc);
  g_rmdir(dir);
  g_free(rc);
  g_free(dir);
}
static void real_apps(void) {
  if (g_strcmp0(g_getenv("ZERO_DOCK_REAL_APPS"), "1")) {
    g_test_skip(
        "Enable with --real-apps; apps run only on the isolated display");
    return;
  }
  const gchar *desktops[] = {"/usr/share/applications/google-chrome.desktop",
                             "/usr/share/applications/thunar.desktop"};
  for (guint app = 0; app < 2; app++) {
    if (!g_file_test(desktops[app], G_FILE_TEST_IS_REGULAR)) {
      g_test_message("Desktop entry unavailable: %s",
                     app ? "Thunar" : "Chrome");
      continue;
    }
    gchar *dir = g_dir_make_tmp("zero-dock-real-app-XXXXXX", NULL),
          *rc = g_build_filename(dir, "settings.rc", NULL);
    GtkWidget *h;
    XfcePanelPlugin *p = host(rc, 9070 + app, &h);
    ZdDock *d = zd_get_dock(p);
    ZdButton *pin = zd_add_pin(d, desktops[app]);
    g_assert_nonnull(pin->app);
    gchar *profile = g_strdup_printf("--user-data-dir=%s/profile", dir);
    gchar *chrome_args[] = {"/usr/bin/google-chrome-stable",
                            profile,
                            "--no-first-run",
                            "--no-default-browser-check",
                            "--disable-background-networking",
                            "--disable-component-update",
                            "--disable-sync",
                            "--disable-extensions",
                            "--disable-gpu",
                            "--password-store=basic",
                            "--new-window",
                            "about:blank",
                            NULL};
    gchar *thunar_args[] = {"/usr/bin/thunar", dir, NULL};
    GPid child;
    g_assert_true(g_spawn_async(NULL, app ? thunar_args : chrome_args, NULL,
                                G_SPAWN_DO_NOT_REAP_CHILD |
                                    G_SPAWN_STDOUT_TO_DEV_NULL |
                                    G_SPAWN_STDERR_TO_DEV_NULL,
                                NULL, NULL, &child, NULL));
    for (guint retry = 0; retry < 60 && !pin->window; retry++)
      spin(100);
    g_assert_nonnull(pin->window);
    g_assert_cmpuint(g_hash_table_size(d->windows), ==, 1);
    zd_activate(pin);
    spin(100);
    zd_minimize(pin);
    spin(100);
    g_assert_true(xfw_window_is_minimized(pin->window));
    zd_activate(pin);
    spin(100);
    g_assert_false(xfw_window_is_minimized(pin->window));
    GPid second;
    g_assert_true(g_spawn_async(NULL, app ? thunar_args : chrome_args, NULL,
                                G_SPAWN_DO_NOT_REAP_CHILD |
                                    G_SPAWN_STDOUT_TO_DEV_NULL |
                                    G_SPAWN_STDERR_TO_DEV_NULL,
                                NULL, NULL, &second, NULL));
    for (guint retry = 0; retry < 50 && g_hash_table_size(d->windows) < 2;
         retry++)
      spin(100);
    g_assert_cmpuint(g_hash_table_size(d->windows), ==, 2);
    zd_close_window(pin);
    spin(400);
    g_assert_nonnull(pin->window);
    g_assert_cmpuint(g_hash_table_size(d->windows), ==, 1);
    zd_close_window(pin);
    spin(400);
    g_assert_null(pin->window);
    g_assert_cmpuint(g_hash_table_size(d->windows), ==, 0);
    kill(child, SIGTERM);
    waitpid(child, NULL, 0);
    g_spawn_close_pid(child);
    waitpid(second, NULL, 0);
    g_spawn_close_pid(second);
    gtk_widget_destroy(h);
    spin(100);
    g_test_message(
        "PASS installed %s: pin reuse, two independent windows, "
        "minimize/restore, transfer after close and return to launcher",
        app ? "Thunar" : "Chrome (disposable profile)");
    g_remove(rc);
    // Browser profile removal is restricted to the temporary directory created
    // here.
    gchar *remove_args[] = {"/usr/bin/rm", "-rf", "--", dir, NULL};
    g_spawn_sync(NULL, remove_args, NULL, 0, NULL, NULL, NULL, NULL, NULL,
                 NULL);
    g_free(profile);
    g_free(rc);
    g_free(dir);
  }
}

int main(int argc, char **argv) {
  if (argc == 2 && g_str_has_prefix(argv[1], "--launch-fixture")) {
    if (g_str_has_prefix(argv[1], "--launch-fixture="))
      g_usleep(
          MIN(g_ascii_strtoull(argv[1] + strlen("--launch-fixture="), NULL, 10),
              5000) *
          1000);
    gtk_init(&argc, &argv);
    GtkWidget *w = fixture("Launched Zero Dock fixture");
    g_signal_connect(w, "destroy", G_CALLBACK(gtk_main_quit), NULL);
    gtk_main();
    return 0;
  }
  g_test_init(&argc, &argv, NULL);
  gtk_init(&argc, &argv);
  g_assert_nonnull(g_getenv("ZERO_DOCK_ISOLATED_TEST"));
  g_test_add_func("/integration/native-all", all);
  g_test_add_func("/integration/external-wrappers", external);
  g_test_add_func("/integration/pinned-lifecycle", pinned_lifecycle);
  g_test_add_func("/integration/launch-feedback", launch_feedback);
  g_test_add_func("/integration/identity-workspaces", identity_workspaces);
  g_test_add_func("/integration/improvements", improvements);
  g_test_add_func("/integration/stress", stress);
  g_test_add_func("/integration/audio-recovery", audio_recovery);
  g_test_add_func("/integration/real-apps", real_apps);
  return g_test_run();
}
