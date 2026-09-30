#include "zero-dock.h"
#include <X11/extensions/XInput2.h>
#include <glib-unix.h>
#include <math.h>

typedef struct {
  gint x_axis, y_axis;
  gdouble x_increment, y_increment;
} ScrollAxes;
struct ZdInput {
  ZdDock *dock;
  Display *display;
  Window root;
  gint opcode;
  guint source;
  GHashTable *axes;
};

static gboolean contains(GtkWidget *w, gint x, gint y) {
  if (!w || !gtk_widget_get_mapped(w))
    return FALSE;
  GtkWidget *top = gtk_widget_get_toplevel(w);
  gint wx, wy, ox, oy;
  GtkAllocation a;
  if (!gtk_widget_translate_coordinates(w, top, 0, 0, &wx, &wy))
    return FALSE;
  gdk_window_get_origin(gtk_widget_get_window(top), &ox, &oy);
  gtk_widget_get_allocation(w, &a);
  return x >= ox + wx && x < ox + wx + a.width && y >= oy + wy &&
         y < oy + wy + a.height;
}
gboolean zd_input_delta(ZdDock *d, gint x, gint y, gdouble dx, gdouble dy,
                        guint32 time) {
  if (d->disposing || d->dragging || !isfinite(dx) || !isfinite(dy) ||
      (dx == 0 && dy == 0))
    return FALSE;
  ZdButton *target = NULL;
  if (d->hover_button && contains(d->preview_sound, x, y))
    target = d->hover_button;
  else
    for (GList *l = d->buttons; l; l = l->next) {
      ZdButton *b = l->data;
      if (contains(b->sound, x, y)) {
        target = b;
        break;
      }
    }
  if (!target)
    return FALSE;
  GdkEventScroll event = {0};
  event.type = GDK_SCROLL;
  event.direction = GDK_SCROLL_SMOOTH;
  event.delta_x = dx;
  event.delta_y = dy;
  event.x_root = x;
  event.y_root = y;
  event.time = time;
  zd_audio_scroll(target, &event);
  return TRUE;
}
static ScrollAxes *scroll_axes(ZdInput *in, gint source) {
  ScrollAxes *axes = g_hash_table_lookup(in->axes, GINT_TO_POINTER(source));
  if (axes)
    return axes;
  axes = g_new0(ScrollAxes, 1);
  axes->x_axis = axes->y_axis = -1;
  gint count;
  XIDeviceInfo *devices = XIQueryDevice(in->display, source, &count);
  if (devices) {
    for (gint j = 0; j < count; j++)
      for (gint i = 0; i < devices[j].num_classes; i++) {
        XIAnyClassInfo *c = devices[j].classes[i];
        if (c->type != XIScrollClass)
          continue;
        XIScrollClassInfo *s = (XIScrollClassInfo *)c;
        if (s->increment == 0)
          continue;
        if (s->scroll_type == XIScrollTypeVertical) {
          axes->y_axis = s->number;
          axes->y_increment = s->increment;
        } else {
          axes->x_axis = s->number;
          axes->x_increment = s->increment;
        }
      }
    XIFreeDeviceInfo(devices);
  }
  g_hash_table_insert(in->axes, GINT_TO_POINTER(source), axes);
  return axes;
}
static void raw_motion(ZdInput *in, XIRawEvent *event) {
  ScrollAxes *axes =
      scroll_axes(in, event->sourceid ? event->sourceid : event->deviceid);
  gdouble dx = 0, dy = 0;
  gdouble *value = event->valuators.values;
  for (gint i = 0; i < event->valuators.mask_len * 8; i++) {
    if (!XIMaskIsSet(event->valuators.mask, i))
      continue;
    if (i == axes->x_axis)
      dx = *value / axes->x_increment;
    if (i == axes->y_axis)
      dy = *value / axes->y_increment;
    value++;
  }
  if (dx == 0 && dy == 0)
    return;
  Window root, child;
  gint x, y, wx, wy;
  guint mask;
  if (XQueryPointer(in->display, in->root, &root, &child, &x, &y, &wx, &wy,
                    &mask))
    zd_input_delta(in->dock, x, y, dx, dy, event->time);
}
static gboolean input_ready(gint fd, GIOCondition condition, gpointer data) {
  (void)fd;
  ZdInput *in = data;
  if (condition & (G_IO_HUP | G_IO_ERR)) {
    in->source = 0;
    return G_SOURCE_REMOVE;
  }
  while (XPending(in->display)) {
    XEvent event;
    XNextEvent(in->display, &event);
    if (event.type != GenericEvent || event.xcookie.extension != in->opcode ||
        !XGetEventData(in->display, &event.xcookie))
      continue;
    if (event.xcookie.evtype == XI_RawMotion)
      raw_motion(in, event.xcookie.data);
    else if (event.xcookie.evtype == XI_HierarchyChanged ||
             event.xcookie.evtype == XI_DeviceChanged)
      g_hash_table_remove_all(in->axes);
    XFreeEventData(in->display, &event.xcookie);
  }
  return G_SOURCE_CONTINUE;
}
ZdInput *zd_input_new(ZdDock *d) {
  ZdInput *in = g_new0(ZdInput, 1);
  in->dock = d;
  in->display = XOpenDisplay(NULL);
  gint event, error, major = 2, minor = 1;
  if (!in->display ||
      !XQueryExtension(in->display, "XInputExtension", &in->opcode, &event,
                       &error) ||
      XIQueryVersion(in->display, &major, &minor) != Success ||
      (major == 2 && minor < 1)) {
    if (in->display)
      XCloseDisplay(in->display);
    g_free(in);
    return NULL;
  }
  in->root = DefaultRootWindow(in->display);
  in->axes = g_hash_table_new_full(g_direct_hash, g_direct_equal, NULL, g_free);
  unsigned char raw[XIMaskLen(XI_LASTEVENT)] = {0},
                change[XIMaskLen(XI_LASTEVENT)] = {0};
  XISetMask(raw, XI_RawMotion);
  XISetMask(change, XI_HierarchyChanged);
  XISetMask(change, XI_DeviceChanged);
  XIEventMask masks[] = {{XIAllMasterDevices, sizeof raw, raw},
                         {XIAllDevices, sizeof change, change}};
  XISelectEvents(in->display, in->root, masks, G_N_ELEMENTS(masks));
  XFlush(in->display);
  in->source = g_unix_fd_add(ConnectionNumber(in->display),
                             G_IO_IN | G_IO_HUP | G_IO_ERR, input_ready, in);
  return in;
}
void zd_input_free(ZdInput *in) {
  if (!in)
    return;
  if (in->source)
    g_source_remove(in->source);
  XCloseDisplay(in->display);
  g_hash_table_destroy(in->axes);
  g_free(in);
}
