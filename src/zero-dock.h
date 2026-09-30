#pragma once
#include <gdk/gdkx.h>
#include <gio/gdesktopappinfo.h>
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
  GDesktopAppInfo *app;
  gchar *desktop, *key;
  GtkWidget *widget, *main, *drawing, *sound;
  GdkPixbuf *icon, *thumbnail;
  ZdAudioStatus audio;
  gboolean pinned, closed;
  guint number;
  gdouble audio_scroll, window_scroll;
  guint32 last_audio_time;
};

struct ZdDock {
  XfcePanelPlugin *plugin;
  XfwScreen *screen;
  GtkWidget *box, *settings, *menu, *show_desktop_item;
  GList *buttons, *apps;
  GHashTable *windows;
  GAppInfoMonitor *app_monitor;
  GtkCssProvider *css;
  ZdAudio *audio;
  ZdInput *input;
  GtkOrientation orientation;
  guint unit, icon_size, preview_width, slots;
  gboolean previews, numbers, all_workspaces, disposing, dragging;
  gchar *rc_path;
  guint refresh_id, active_frame_id;
  ZdButton *drag_button, *insert_button;
  gboolean insert_after;
  ZdButton *hover_button;
  guint hover_id, preview_tick, leave_id;
  GtkWidget *preview, *preview_image, *preview_title, *preview_sound,
      *preview_status;
  gboolean autohide_blocked;
  GtkWidget *bubble, *bubble_text, *bubble_bar;
  guint bubble_id;
};

void zd_construct(XfcePanelPlugin *plugin);
ZdDock *zd_get_dock(XfcePanelPlugin *plugin);
void zd_queue_refresh(ZdDock *dock);
void zd_refresh(ZdDock *dock);
void zd_save(ZdDock *dock);
void zd_configure(XfcePanelPlugin *plugin, ZdDock *dock);
void zd_save_pins_to_keyfile(ZdDock *dock, GKeyFile *file);
GDesktopAppInfo *zd_match_app(ZdDock *dock, XfwWindow *window);
void zd_launch(ZdButton *button);
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
void zd_button_free(ZdButton *button);
void zd_update_buttons(ZdDock *dock);

void zd_window_menu(ZdButton *button, GdkEvent *event);
void zd_pin_menu(ZdButton *button, GdkEvent *event);
void zd_menu_install(ZdDock *dock);
void zd_menu_sync(ZdDock *dock);
void zd_minimize_all(ZdDock *dock);

GdkPixbuf *zd_capture(ZdButton *button);
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
