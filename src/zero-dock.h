#pragma once
#include "config.h"
#include <gdk/gdkx.h>
#include <gio/gdesktopappinfo.h>
#include <glib/gi18n-lib.h>
#include <gtk/gtk.h>
#include <libxfce4panel/libxfce4panel.h>
#include <libxfce4ui/libxfce4ui.h>
#include <libxfce4windowing/libxfce4windowing.h>
#include <libxfce4windowing/xfw-x11.h>
#include <pulse/pulseaudio.h>

typedef struct ZdDock ZdDock;
typedef struct ZdButton ZdButton;
typedef struct ZdAudio ZdAudio;
typedef struct ZdInput ZdInput;

typedef struct {
  guint32 index;
  pid_t pid;
  gchar *binary, *name;
  gboolean mute, corked;
  pa_cvolume volume;
} ZdStream;

typedef struct {
  gboolean present, muted, playing;
  guint percent;
} ZdAudioStatus;

struct ZdButton {
  ZdDock *dock;
  XfwWindow *window;
  GdkWindow *xwindow;
  GDesktopAppInfo *app;
  gchar *desktop, *key;
  GtkWidget *widget, *main, *drawing, *sound;
  GdkPixbuf *icon, *thumbnail;
  ZdAudioStatus audio;
  gboolean pinned, closed, icon_dirty, match_dirty, eligible;
  guint number, icon_pixels, icon_scale, match_generation;
  gboolean launching;
  gint64 launch_until, thumbnail_time;
  pid_t launch_pid, pid;
  gchar *startup_id, *launch_error;
  gdouble audio_scroll, window_scroll;
  guint32 last_audio_time;
};

struct ZdDock {
  XfcePanelPlugin *plugin;
  XfwScreen *screen;
  GtkWidget *container, *box, *overflow, *association_dialog, *settings, *menu,
      *show_desktop_item, *error_dialog, *about_dialog;
  GList *buttons, *apps;
  GHashTable *windows, *associations;
  GAppInfoMonitor *app_monitor;
  GtkIconTheme *icon_theme;
  GtkCssProvider *css;
  ZdAudio *audio;
  ZdInput *input;
  GtkOrientation orientation;
  guint unit, icon_size, preview_width, preview_delay, preview_interval,
      launch_timeout, slots, max_visible, left_action, middle_action,
      app_generation;
  gboolean previews, numbers, all_workspaces, scroll_windows, disposing,
      dragging, save_blocked;
  gchar *rc_path;
  guint refresh_id, active_frame_id, launch_tick;
  Atom identity_atoms[4];
  ZdButton *drag_button, *insert_button;
  gboolean insert_after;
  ZdButton *hover_button;
  guint hover_id, preview_tick, leave_id;
  GtkWidget *preview, *preview_image, *preview_title, *preview_sound,
      *preview_status, *preview_workspace, *preview_close;
  gboolean autohide_blocked;
  GtkWidget *bubble, *bubble_text, *bubble_bar;
  guint bubble_id;
};

void zd_construct(XfcePanelPlugin *plugin);
ZdDock *zd_get_dock(XfcePanelPlugin *plugin);
void zd_queue_refresh(ZdDock *dock);
void zd_refresh(ZdDock *dock);
gboolean zd_save(ZdDock *dock);
void zd_configure(XfcePanelPlugin *plugin, ZdDock *dock);
void zd_settings_defaults(ZdDock *dock);
void zd_settings_load(ZdDock *dock, GKeyFile *file);
gboolean zd_settings_read(ZdDock *dock, const gchar *path, GKeyFile *file);
gchar *zd_diagnostics(ZdDock *dock);
void zd_show_error(ZdDock *dock, const gchar *message);
void zd_reload_apps(ZdDock *dock);
void zd_save_pins_to_keyfile(ZdDock *dock, GKeyFile *file);
GDesktopAppInfo *zd_match_app(ZdDock *dock, XfwWindow *window);
gint zd_app_match_score(GDesktopAppInfo *app, const gchar *const *class_ids);
gboolean zd_app_equal(GDesktopAppInfo *a, GDesktopAppInfo *b);
void zd_launch(ZdButton *button);
void zd_launch_complete(ZdButton *button);
gchar *zd_window_property(XfwWindow *window, const gchar *name);
void zd_update_audio_buttons(ZdDock *dock);
gboolean zd_window_in_workspace(XfwWindow *window);
void zd_pin_app(ZdDock *dock, GDesktopAppInfo *app);
void zd_unpin(ZdButton *button);
void zd_move_button(ZdDock *dock, ZdButton *source, ZdButton *target,
                    gboolean after);
guint32 zd_timestamp(ZdDock *dock);
pid_t zd_window_pid(XfwWindow *window);
void zd_activate(ZdButton *button);
void zd_toggle(ZdButton *button);
void zd_minimize(ZdButton *button);
void zd_cycle(ZdDock *dock, gint delta);
ZdButton *zd_add_pin(ZdDock *dock, const gchar *desktop);
ZdButton *zd_add_window(ZdDock *dock, XfwWindow *window);
void zd_button_attach_window(ZdButton *button, XfwWindow *window);
void zd_button_release_window(ZdButton *button);
void zd_button_free(ZdButton *button);
void zd_update_buttons(ZdDock *dock);

void zd_window_menu(ZdButton *button, GdkEvent *event);
void zd_pin_menu(ZdButton *button, GdkEvent *event);
void zd_menu_install(ZdDock *dock);
void zd_menu_sync(ZdDock *dock);
void zd_minimize_all(ZdDock *dock);

GdkPixbuf *zd_capture(ZdButton *button);
void zd_trim_frames(ZdDock *dock, ZdButton *keep);
void zd_preview_schedule(ZdButton *button);
void zd_preview_hide(ZdDock *dock);
void zd_preview_maybe_leave(ZdDock *dock);
void zd_preview_update_audio(ZdDock *dock);
void zd_volume_bubble(ZdButton *button);
void zd_popups_dispose(ZdDock *dock);

ZdAudio *zd_audio_new(ZdDock *dock);
void zd_audio_free(ZdAudio *audio);
ZdAudioStatus zd_audio_status(ZdButton *button);
void zd_audio_mute(ZdButton *button);
void zd_audio_volume(ZdButton *button, gint steps);
void zd_audio_scroll(ZdButton *button, GdkEventScroll *event);
ZdInput *zd_input_new(ZdDock *dock);
void zd_input_free(ZdInput *input);
gboolean zd_input_delta(ZdDock *dock, gint x, gint y, gdouble dx, gdouble dy,
                        guint32 time);
gboolean zd_pid_descends(pid_t pid, pid_t ancestor);
gboolean zd_fuzzy_match(const gchar *a, const gchar *b);
gboolean zd_stream_matches(ZdButton *button, const ZdStream *stream);

/* Explicit class rules take precedence over automatic desktop identification.
 */
gchar *zd_association_key(XfwWindow *window);
void zd_associations_load(ZdDock *dock, GKeyFile *file);
void zd_associations_save(ZdDock *dock, GKeyFile *file);
gboolean zd_associate(ZdDock *dock, XfwWindow *window, const gchar *path,
                      GError **error);
void zd_association_choose(ZdButton *button);
void zd_association_clear(ZdButton *button);
void zd_overflow_menu(ZdDock *dock, GdkEvent *event);
void zd_layout_update(ZdDock *dock);
void zd_close_window(ZdButton *button);
gboolean zd_config_export(ZdDock *dock, const gchar *path, GError **error);
gboolean zd_config_import(ZdDock *dock, const gchar *path, GError **error);

void zd_popup_menu(ZdDock *dock, GtkWidget *menu, GtkWidget *anchor,
                   GdkEvent *event);

gboolean zd_focus_key(GtkWidget *widget, GdkEventKey *event, ZdDock *dock);
