#include "../src/zero-dock.h"
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
int main(int argc, char **argv) {
  g_test_init(&argc, &argv, NULL);
  g_test_add_func("/audio/ancestry", ancestry);
  g_test_add_func("/audio/matching", matching);
  g_test_add_func("/settings/pins-filter-order", pins);
  g_test_add_func("/native/registration", registration);
  return g_test_run();
}
