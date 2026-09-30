#include "zero-dock.h"
#include <errno.h>
#include <math.h>
#include <pulse/glib-mainloop.h>
#include <stdio.h>
#include <unistd.h>
struct ZdAudio {
  ZdDock *dock;
  pa_glib_mainloop *loop;
  pa_context *ctx;
  GPtrArray *streams, *pending;
  guint query_id, reconnect_id;
  gboolean querying, again, closing;
};
static void stream_free(gpointer p) {
  ZdStream *s = p;
  g_free(s->binary);
  g_free(s->name);
  g_free(s);
}
gboolean zd_pid_descends(pid_t pid, pid_t ancestor) {
  if (pid <= 0 || ancestor <= 0)
    return FALSE;
  for (guint depth = 0; depth < 96 && pid > 1; depth++) {
    if (pid == ancestor)
      return TRUE;
    gchar *path = g_strdup_printf("/proc/%ld/stat", (long)pid), *data = NULL;
    gboolean ok = g_file_get_contents(path, &data, NULL, NULL);
    g_free(path);
    if (!ok)
      return FALSE;
    gchar *end = strrchr(data, ')');
    long parent = 0;
    char state;
    gboolean parsed = end && sscanf(end + 1, " %c %ld", &state, &parent) == 2;
    g_free(data);
    if (!parsed || parent <= 0 || parent == pid)
      return FALSE;
    pid = (pid_t)parent;
  }
  return pid == ancestor;
}
static gchar *normalize(const gchar *s) {
  if (!s || !*s)
    return NULL;
  gchar *base = g_path_get_basename(s), *n = g_ascii_strdown(base, -1);
  g_free(base);
  if (g_str_has_suffix(n, ".desktop"))
    n[strlen(n) - 8] = '\0';
  if (g_str_has_suffix(n, ".exe"))
    n[strlen(n) - 4] = '\0';
  return n;
}
gboolean zd_fuzzy_match(const gchar *a, const gchar *b) {
  gchar *x = normalize(a), *y = normalize(b);
  gboolean ok = x && y && *x && *y &&
                (!strcmp(x, y) || (MIN(strlen(x), strlen(y)) >= 4 &&
                                   (strstr(x, y) || strstr(y, x))));
  g_free(x);
  g_free(y);
  return ok;
}
gboolean zd_stream_matches(ZdButton *b, const ZdStream *s) {
  if (b->window && s->pid > 0) {
    pid_t pid = zd_window_pid(b->window);
    if (zd_pid_descends(s->pid, pid))
      return TRUE;
    for (GList *l = b->dock->buttons; l; l = l->next) {
      ZdButton *q = l->data;
      if (q->window && zd_pid_descends(s->pid, zd_window_pid(q->window)))
        return FALSE;
    }
  }
  if (b->window) {
    const gchar *const *ids = xfw_window_get_class_ids(b->window);
    for (guint i = 0; ids && ids[i]; i++)
      if (zd_fuzzy_match(ids[i], s->binary))
        return TRUE;
  }
  if (b->app) {
    const gchar *exe = g_app_info_get_executable(G_APP_INFO(b->app)),
                *wm = g_desktop_app_info_get_startup_wm_class(b->app);
    if (zd_fuzzy_match(exe, s->binary) || zd_fuzzy_match(wm, s->binary))
      return TRUE;
  }
  return FALSE;
}
ZdAudioStatus zd_audio_status(ZdButton *b) {
  ZdAudioStatus r = {0};
  ZdAudio *a = b->dock->audio;
  if (!a)
    return r;
  r.muted = TRUE;
  for (guint j = 0; j < a->streams->len; j++) {
    ZdStream *s = g_ptr_array_index(a->streams, j);
    if (zd_stream_matches(b, s)) {
      r.present = TRUE;
      r.muted &= s->mute;
      r.playing |= !s->corked;
      r.percent =
          MAX(r.percent, (guint)llround(100. * pa_cvolume_avg(&s->volume) /
                                        PA_VOLUME_NORM));
    }
  }
  if (!r.present)
    r.muted = FALSE;
  return r;
}
static void query_schedule(ZdAudio *a);
static void info_cb(pa_context *c, const pa_sink_input_info *i, int end,
                    void *data) {
  (void)c;
  ZdAudio *a = data;
  if (a->closing)
    return;
  if (end) {
    a->querying = FALSE;
    if (end > 0) {
      GPtrArray *old = a->streams;
      a->streams = a->pending;
      a->pending = NULL;
      g_ptr_array_free(old, TRUE);
      zd_queue_refresh(a->dock);
    } else
      g_clear_pointer(&a->pending, g_ptr_array_unref);
    if (a->again) {
      a->again = FALSE;
      query_schedule(a);
    }
    return;
  }
  ZdStream *s = g_new0(ZdStream, 1);
  s->index = i->index;
  s->mute = i->mute;
  s->corked = i->corked;
  s->volume = i->volume;
  const gchar *pid =
      pa_proplist_gets(i->proplist, PA_PROP_APPLICATION_PROCESS_ID);
  if (pid) {
    gchar *endp;
    gint64 p = g_ascii_strtoll(pid, &endp, 10);
    if (!*endp && p > 0 && p <= G_MAXINT)
      s->pid = p;
  }
  s->binary = g_strdup(
      pa_proplist_gets(i->proplist, PA_PROP_APPLICATION_PROCESS_BINARY));
  s->name = g_strdup(pa_proplist_gets(i->proplist, PA_PROP_APPLICATION_NAME));
  g_ptr_array_add(a->pending, s);
}
static gboolean query(gpointer p) {
  ZdAudio *a = p;
  a->query_id = 0;
  if (a->closing || !a->ctx || pa_context_get_state(a->ctx) != PA_CONTEXT_READY)
    return G_SOURCE_REMOVE;
  if (a->querying) {
    a->again = TRUE;
    return G_SOURCE_REMOVE;
  }
  a->querying = TRUE;
  a->pending = g_ptr_array_new_with_free_func(stream_free);
  pa_operation *o = pa_context_get_sink_input_info_list(a->ctx, info_cb, a);
  if (o)
    pa_operation_unref(o);
  else {
    a->querying = FALSE;
    g_clear_pointer(&a->pending, g_ptr_array_unref);
  }
  return G_SOURCE_REMOVE;
}
static void query_schedule(ZdAudio *a) {
  if (!a->closing && !a->query_id)
    a->query_id = g_timeout_add(35, query, a);
}
static void subscription(pa_context *c, pa_subscription_event_type_t t,
                         uint32_t index, void *p) {
  (void)c;
  (void)index;
  if ((t & PA_SUBSCRIPTION_EVENT_FACILITY_MASK) ==
      PA_SUBSCRIPTION_EVENT_SINK_INPUT)
    query_schedule(p);
}
static void connect_audio(ZdAudio *a);
static gboolean reconnect(gpointer p) {
  ZdAudio *a = p;
  a->reconnect_id = 0;
  connect_audio(a);
  return G_SOURCE_REMOVE;
}
static void context_state(pa_context *c, void *p) {
  ZdAudio *a = p;
  if (a->closing)
    return;
  switch (pa_context_get_state(c)) {
  case PA_CONTEXT_READY:
    pa_context_set_subscribe_callback(c, subscription, a);
    {
      pa_operation *o =
          pa_context_subscribe(c, PA_SUBSCRIPTION_MASK_SINK_INPUT, NULL, NULL);
      if (o)
        pa_operation_unref(o);
    }
    query_schedule(a);
    break;
  case PA_CONTEXT_FAILED:
  case PA_CONTEXT_TERMINATED:
    g_ptr_array_set_size(a->streams, 0);
    g_clear_pointer(&a->pending, g_ptr_array_unref);
    a->querying = FALSE;
    zd_queue_refresh(a->dock);
    if (!a->reconnect_id)
      a->reconnect_id = g_timeout_add_seconds(3, reconnect, a);
    break;
  default:
    break;
  }
}
static void connect_audio(ZdAudio *a) {
  if (a->ctx) {
    pa_context_set_state_callback(a->ctx, NULL, NULL);
    pa_context_disconnect(a->ctx);
    pa_context_unref(a->ctx);
  }
  a->ctx = pa_context_new(pa_glib_mainloop_get_api(a->loop), "Zero Dock");
  pa_context_set_state_callback(a->ctx, context_state, a);
  if (pa_context_connect(a->ctx, NULL, PA_CONTEXT_NOAUTOSPAWN, NULL) < 0 &&
      !a->reconnect_id)
    a->reconnect_id = g_timeout_add_seconds(3, reconnect, a);
}
ZdAudio *zd_audio_new(ZdDock *d) {
  ZdAudio *a = g_new0(ZdAudio, 1);
  a->dock = d;
  a->streams = g_ptr_array_new_with_free_func(stream_free);
  a->loop = pa_glib_mainloop_new(NULL);
  connect_audio(a);
  return a;
}
void zd_audio_free(ZdAudio *a) {
  if (!a)
    return;
  a->closing = TRUE;
  if (a->query_id)
    g_source_remove(a->query_id);
  if (a->reconnect_id)
    g_source_remove(a->reconnect_id);
  if (a->ctx) {
    pa_context_set_state_callback(a->ctx, NULL, NULL);
    pa_context_set_subscribe_callback(a->ctx, NULL, NULL);
    pa_context_disconnect(a->ctx);
    pa_context_unref(a->ctx);
  }
  pa_glib_mainloop_free(a->loop);
  g_ptr_array_free(a->streams, TRUE);
  g_clear_pointer(&a->pending, g_ptr_array_unref);
  g_free(a);
}
void zd_audio_mute(ZdButton *b) {
  ZdAudio *a = b->dock->audio;
  if (!a || !a->ctx || pa_context_get_state(a->ctx) != PA_CONTEXT_READY)
    return;
  gboolean mute = !zd_audio_status(b).muted;
  for (guint j = 0; j < a->streams->len; j++) {
    ZdStream *s = g_ptr_array_index(a->streams, j);
    if (zd_stream_matches(b, s)) {
      pa_operation *o =
          pa_context_set_sink_input_mute(a->ctx, s->index, mute, NULL, NULL);
      if (o) {
        pa_operation_unref(o);
        s->mute = mute;
      }
    }
  }
  zd_update_buttons(b->dock);
}
void zd_audio_volume(ZdButton *b, gint steps) {
  ZdAudio *a = b->dock->audio;
  if (!a || !a->ctx || pa_context_get_state(a->ctx) != PA_CONTEXT_READY)
    return;
  for (guint j = 0; j < a->streams->len; j++) {
    ZdStream *s = g_ptr_array_index(a->streams, j);
    if (zd_stream_matches(b, s)) {
      pa_volume_t old = pa_cvolume_avg(&s->volume);
      pa_volume_t v = CLAMP((gint64)old + (gint64)steps * PA_VOLUME_NORM / 20,
                            0, 2 * PA_VOLUME_NORM);
      pa_cvolume volume = s->volume;
      pa_cvolume_scale(&volume, v);
      pa_operation *o = pa_context_set_sink_input_volume(a->ctx, s->index,
                                                         &volume, NULL, NULL);
      if (o) {
        pa_operation_unref(o);
        s->volume = volume;
      }
    }
  }
  zd_update_buttons(b->dock);
}
