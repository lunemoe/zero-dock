#include "../src/zero-dock.h"
#include <glib/gstdio.h>
#include <unistd.h>
static void ancestry(void) {
  g_assert_true(zd_pid_descends(getpid(), getpid()));
  g_assert_true(zd_pid_descends(getpid(), getppid()));
  g_assert_false(zd_pid_descends(getpid(), 0));
  g_assert_false(zd_pid_descends(0, getpid()));
  g_assert_false(zd_pid_descends(getpid(), 2147483647));
}
static void matching(void) {
  g_assert_true(zd_fuzzy_match("/usr/bin/google-chrome", "Google-chrome"));
  g_assert_true(zd_fuzzy_match("org.mozilla.firefox.desktop", "firefox"));
  g_assert_true(zd_fuzzy_match("game.exe", "game"));
  g_assert_false(zd_fuzzy_match("sh", "shotwell"));
  g_assert_false(zd_fuzzy_match(NULL, "chrome"));
  g_assert_false(zd_fuzzy_match("", ""));
  g_assert_false(zd_fuzzy_match("firefox", "chromium"));
}
static void desktop_identity(void) {
  gchar *dir = g_dir_make_tmp("zero-dock-apps-XXXXXX", NULL);
  gchar *first = g_build_filename(dir, "first.desktop", NULL),
        *second = g_build_filename(dir, "second.desktop", NULL);
  const gchar *entry = "[Desktop Entry]\nType=Application\nName=Fixture\n"
                       "Exec=/usr/bin/true\nStartupWMClass=FirstFixture\n";
  g_assert_true(g_file_set_contents(first, entry, -1, NULL));
  g_assert_true(g_file_set_contents(second, entry, -1, NULL));
  GDesktopAppInfo *a = g_desktop_app_info_new_from_filename(first),
                  *copy = g_desktop_app_info_new_from_filename(first),
                  *b = g_desktop_app_info_new_from_filename(second);
  g_assert_nonnull(a);
  g_assert_nonnull(copy);
  g_assert_nonnull(b);
  g_assert_true(zd_app_equal(a, copy));
  g_assert_false(zd_app_equal(a, b));
  g_assert_false(zd_app_equal(a, NULL));
  const gchar *exact[] = {"firstfixture", NULL}, *executable[] = {"true", NULL},
              *unrelated[] = {"OtherFixture", NULL};
  g_assert_cmpint(zd_app_match_score(a, exact), >,
                  zd_app_match_score(a, executable));
  g_assert_cmpint(zd_app_match_score(a, unrelated), ==, 0);
  g_assert_cmpint(zd_app_match_score(a, NULL), ==, 0);
  g_object_unref(a);
  g_object_unref(copy);
  g_object_unref(b);
  g_remove(first);
  g_remove(second);
  g_rmdir(dir);
  g_free(first);
  g_free(second);
  g_free(dir);
}
static void pins(void) {
  ZdDock d = {0};
  ZdButton a = {.pinned = TRUE, .desktop = "/tmp/a.desktop"},
           b = {.pinned = FALSE},
           c = {.pinned = TRUE, .desktop = "/tmp/c.desktop"};
  d.buttons = g_list_append(d.buttons, &a);
  d.buttons = g_list_append(d.buttons, &b);
  d.buttons = g_list_append(d.buttons, &c);
  GKeyFile *f = g_key_file_new();
  zd_save_pins_to_keyfile(&d, f);
  gsize n = 0;
  gchar **v = g_key_file_get_string_list(f, "Dock", "Pinned", &n, NULL);
  g_assert_cmpuint(n, ==, 2);
  g_assert_cmpstr(v[0], ==, a.desktop);
  g_assert_cmpstr(v[1], ==, c.desktop);
  g_strfreev(v);
  g_key_file_unref(f);
  g_list_free(d.buttons);
}
static void registration(void) {
  GKeyFile *f = g_key_file_new();
  g_assert_true(
      g_key_file_load_from_file(f, TEST_DESKTOP, G_KEY_FILE_NONE, NULL));
  g_assert_true(g_key_file_has_group(f, "Xfce Panel"));
  g_assert_false(
      g_key_file_get_boolean(f, "Xfce Panel", "X-XFCE-Internal", NULL));
  g_assert_false(
      g_key_file_get_boolean(f, "Xfce Panel", "X-XFCE-Unique", NULL));
  g_assert_true(
      g_key_file_get_boolean(f, "Xfce Panel", "X-XFCE-Supports-X11", NULL));
  gchar *module = g_key_file_get_string(f, "Xfce Panel", "X-XFCE-Module", NULL);
  g_assert_cmpstr(module, ==, "zero-dock");
  g_free(module);
  g_key_file_unref(f);
}
static void invalid_settings(void) {
  ZdDock d = {0};
  zd_settings_defaults(&d);
  GKeyFile *f = g_key_file_new();
  g_assert_true(g_key_file_load_from_data(
      f,
      "[Dock]\nPreviews=broken\nNumbers=false\nSlots=-4\n"
      "PreviewWidth=bad\nPreviewDelay=99999\nPreviewInterval=1\n"
      "LaunchTimeout=bad\nLeftAction=-2\nMiddleAction=99\nMaxVisible=-"
      "5\nScrollWindows=bad\n",
      -1, G_KEY_FILE_NONE, NULL));
  zd_settings_load(&d, f);
  g_assert_true(d.previews);
  g_assert_false(d.numbers);
  g_assert_cmpuint(d.slots, ==, 0);
  g_assert_cmpuint(d.preview_width, ==, 300);
  g_assert_cmpuint(d.preview_delay, ==, 2000);
  g_assert_cmpuint(d.preview_interval, ==, 200);
  g_assert_cmpuint(d.launch_timeout, ==, 10000);
  g_assert_cmpuint(d.left_action, ==, 0);
  g_assert_cmpuint(d.middle_action, ==, 2);
  g_assert_cmpuint(d.max_visible, ==, 0);
  g_assert_true(d.scroll_windows);
  g_key_file_unref(f);
}
int main(int argc, char **argv) {
  g_test_init(&argc, &argv, NULL);
  g_test_add_func("/audio/ancestry", ancestry);
  g_test_add_func("/audio/matching", matching);
  g_test_add_func("/applications/desktop-identity", desktop_identity);
  g_test_add_func("/settings/pins-filter-order", pins);
  g_test_add_func("/settings/invalid-values", invalid_settings);
  g_test_add_func("/native/registration", registration);
  return g_test_run();
}
