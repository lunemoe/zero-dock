#include "../src/zero-dock.h"
#include <X11/Xutil.h>
#include <X11/extensions/XTest.h>
#include <glib/gstdio.h>
#include <gtk/gtkx.h>
#include <libxfce4panel/xfce-panel-plugin-provider.h>
#include <signal.h>
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
  g_assert_cmpuint(g_list_length(d->buttons), ==, 3);
  g_test_message("PASS real desktop-file URI drag pins a launcher");
  GDesktopAppInfo *app = g_desktop_app_info_new_from_filename(desktop);
  g_set_object(&a->app, app);
  g_set_object(&b->app, app);
  zd_pin_app(d, app);
  zd_pin_app(d, app);
  g_assert_cmpuint(g_list_length(d->buttons), ==, 3);
  ZdButton *pin = g_list_last(d->buttons)->data;
  g_assert_true(pin->pinned);
  g_assert_cmpuint(g_list_length(d2->buttons), ==, 2);
  zd_move_button(d, pin, a, FALSE);
  g_assert_true(d->buttons->data == pin);
  zd_move_button(d, b, a, FALSE);
  g_assert_true(d->buttons->next->data == b);
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
  zd_preview_schedule(pin);
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
  g_test_message("PASS window/workspace menu, desktop actions menu, adjacent "
                 "launcher closes preview, tooltip fallback");
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
int main(int argc, char **argv) {
  g_test_init(&argc, &argv, NULL);
  gtk_init(&argc, &argv);
  g_assert_nonnull(g_getenv("ZERO_DOCK_ISOLATED_TEST"));
  g_test_add_func("/integration/native-all", all);
  g_test_add_func("/integration/external-wrappers", external);
  return g_test_run();
}
