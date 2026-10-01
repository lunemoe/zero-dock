#include "zero-dock.h"
#include <glib/gstdio.h>
#include <sys/stat.h>

gboolean zd_settings_read(ZdDock *d, const gchar *path, GKeyFile *file) {
  GError *error = NULL;
  if (g_key_file_load_from_file(file, path, G_KEY_FILE_KEEP_COMMENTS, &error))
    return TRUE;
  if (!g_error_matches(error, G_FILE_ERROR, G_FILE_ERROR_NOENT)) {
    gchar *contents = NULL;
    gsize length;
    gchar *backup = g_strdup_printf("%s.invalid-%" G_GINT64_FORMAT, d->rc_path,
                                    g_get_real_time());
    gboolean saved =
        g_file_get_contents(path, &contents, &length, NULL) &&
        g_file_set_contents_full(backup, contents, length,
                                 G_FILE_SET_CONTENTS_CONSISTENT, 0600, NULL);
    d->save_blocked = !saved;
    zd_show_error(
        d,
        saved
            ? _("配置文件无法读取，已保留原文件备份并使用默认设置。")
            : _("配置文件无法读取，设置暂不保存。请修复配置后重新加载插件。"));
    g_free(contents);
    g_free(backup);
  }
  g_clear_error(&error);
  return FALSE;
}
void zd_settings_defaults(ZdDock *d) {
  d->previews = d->numbers = d->all_workspaces = TRUE;
  d->preview_width = 300;
  d->preview_delay = 350;
  d->preview_interval = 650;
  d->launch_timeout = 10000;
  d->slots = 10;
  d->max_visible = 16;
  d->left_action = d->middle_action = 0;
  d->scroll_windows = TRUE;
}
static gint integer_setting(GKeyFile *f, const gchar *key, gint fallback,
                            gint low, gint high) {
  GError *error = NULL;
  gint value = g_key_file_get_integer(f, "Dock", key, &error);
  if (error) {
    g_clear_error(&error);
    return fallback;
  }
  return CLAMP(value, low, high);
}
static gboolean boolean_setting(GKeyFile *f, const gchar *key,
                                gboolean fallback) {
  GError *error = NULL;
  gboolean value = g_key_file_get_boolean(f, "Dock", key, &error);
  if (error) {
    g_clear_error(&error);
    return fallback;
  }
  return value;
}
void zd_settings_load(ZdDock *d, GKeyFile *f) {
  d->previews = boolean_setting(f, "Previews", d->previews);
  d->numbers = boolean_setting(f, "Numbers", d->numbers);
  d->all_workspaces = boolean_setting(f, "AllWorkspaces", d->all_workspaces);
  d->preview_width =
      integer_setting(f, "PreviewWidth", d->preview_width, 180, 600);
  d->preview_delay =
      integer_setting(f, "PreviewDelay", d->preview_delay, 100, 2000);
  d->preview_interval =
      integer_setting(f, "PreviewInterval", d->preview_interval, 200, 2000);
  d->launch_timeout =
      integer_setting(f, "LaunchTimeout", d->launch_timeout, 2000, 60000);
  d->slots = integer_setting(f, "Slots", d->slots, 0, 32);
  d->max_visible = integer_setting(f, "MaxVisible", d->max_visible, 0, 64);
  d->left_action = integer_setting(f, "LeftAction", d->left_action, 0, 1);
  d->middle_action = integer_setting(f, "MiddleAction", d->middle_action, 0, 2);
  d->scroll_windows = boolean_setting(f, "ScrollWindows", d->scroll_windows);
}
gchar *zd_diagnostics(ZdDock *d) {
  guint pins = 0, missing = 0;
  for (GList *l = d->buttons; l; l = l->next) {
    ZdButton *b = l->data;
    pins += b->pinned;
    missing += b->pinned && !b->app;
  }
  return g_strdup_printf(
      "Zero Dock %s\nBackend: X11\nGTK: %u.%u.%u\n"
      "Windows: %u\nPinned: %u\nUnavailable launchers: %u\n"
      "Orientation: %s\nIcon size: %u\nScale: %d\n"
      "Previews: %d\nPreview width: %u\nPreview delay: %u ms\n"
      "Preview interval: %u ms\nLaunch timeout: %u ms\n"
      "All workspaces: %d\nReserved slots: %u\nMax visible: %u\nManual "
      "associations: %u\n",
      ZERO_DOCK_VERSION, gtk_get_major_version(), gtk_get_minor_version(),
      gtk_get_micro_version(), g_hash_table_size(d->windows), pins, missing,
      d->orientation == GTK_ORIENTATION_HORIZONTAL ? "horizontal" : "vertical",
      d->icon_size, gtk_widget_get_scale_factor(d->box), d->previews,
      d->preview_width, d->preview_delay, d->preview_interval,
      d->launch_timeout, d->all_workspaces, d->slots, d->max_visible,
      g_hash_table_size(d->associations));
}
static void error_destroy(GtkWidget *w, ZdDock *d) {
  (void)w;
  d->error_dialog = NULL;
}
void zd_show_error(ZdDock *d, const gchar *message) {
  if (d->error_dialog)
    gtk_widget_destroy(d->error_dialog);
  d->error_dialog =
      gtk_message_dialog_new(d->settings ? GTK_WINDOW(d->settings) : NULL,
                             GTK_DIALOG_DESTROY_WITH_PARENT, GTK_MESSAGE_ERROR,
                             GTK_BUTTONS_CLOSE, "%s", message);
  gtk_window_set_title(GTK_WINDOW(d->error_dialog), _("Zero Dock 提示"));
  gtk_window_set_skip_taskbar_hint(GTK_WINDOW(d->error_dialog), TRUE);
  xfce_panel_plugin_take_window(d->plugin, GTK_WINDOW(d->error_dialog));
  g_signal_connect(d->error_dialog, "destroy", G_CALLBACK(error_destroy), d);
  g_signal_connect_swapped(d->error_dialog, "response",
                           G_CALLBACK(gtk_widget_destroy), d->error_dialog);
  gtk_widget_show(d->error_dialog);
}
static void settings_changed(GtkToggleButton *b, ZdDock *d) {
  const gchar *k = g_object_get_data(G_OBJECT(b), "setting");
  gboolean v = gtk_toggle_button_get_active(b);
  if (!strcmp(k, "previews"))
    d->previews = v;
  else if (!strcmp(k, "numbers"))
    d->numbers = v;
  else if (!strcmp(k, "workspaces"))
    d->all_workspaces = v;
  else
    d->scroll_windows = v;
  zd_preview_hide(d);
  zd_save(d);
  zd_refresh(d);
}
static void value_changed(GtkSpinButton *spin, ZdDock *d) {
  guint *setting = g_object_get_data(G_OBJECT(spin), "setting");
  *setting = gtk_spin_button_get_value_as_int(spin);
  zd_preview_hide(d);
  if (setting == &d->preview_width)
    for (GList *l = d->buttons; l; l = l->next)
      g_clear_object(&((ZdButton *)l->data)->thumbnail);
  zd_save(d);
  zd_refresh(d);
}
static void settings_destroy(GtkWidget *w, ZdDock *d) {
  (void)w;
  d->settings = NULL;
}
static void reset_settings(GtkButton *button, ZdDock *d) {
  (void)button;
  zd_settings_defaults(d);
  zd_preview_hide(d);
  for (GList *l = d->buttons; l; l = l->next)
    g_clear_object(&((ZdButton *)l->data)->thumbnail);
  zd_save(d);
  zd_refresh(d);
  gtk_widget_destroy(d->settings);
  zd_configure(d->plugin, d);
}
static void export_response(GtkDialog *dialog, gint response, ZdDock *d) {
  if (response == GTK_RESPONSE_ACCEPT) {
    gchar *path = gtk_file_chooser_get_filename(GTK_FILE_CHOOSER(dialog));
    gchar *text = zd_diagnostics(d);
    GError *error = NULL;
    if (!g_file_set_contents_full(
            path, text, -1, G_FILE_SET_CONTENTS_CONSISTENT, 0600, &error)) {
      zd_show_error(d, error->message);
      g_clear_error(&error);
    }
    g_free(path);
    g_free(text);
  }
  gtk_widget_destroy(GTK_WIDGET(dialog));
}
static void export_diagnostics(GtkButton *button, ZdDock *d) {
  (void)button;
  GtkWidget *dialog = gtk_file_chooser_dialog_new(
      _("导出诊断信息"), GTK_WINDOW(d->settings), GTK_FILE_CHOOSER_ACTION_SAVE,
      _("取消"), GTK_RESPONSE_CANCEL, _("保存"), GTK_RESPONSE_ACCEPT, NULL);
  gtk_file_chooser_set_current_name(GTK_FILE_CHOOSER(dialog),
                                    "zero-dock-diagnostics.txt");
  gtk_file_chooser_set_do_overwrite_confirmation(GTK_FILE_CHOOSER(dialog),
                                                 TRUE);
  gtk_window_set_destroy_with_parent(GTK_WINDOW(dialog), TRUE);
  gtk_window_set_skip_taskbar_hint(GTK_WINDOW(dialog), TRUE);
  g_signal_connect(dialog, "response", G_CALLBACK(export_response), d);
  gtk_widget_show(dialog);
}
static void click_changed(GtkComboBox *combo, ZdDock *d) {
  guint *field = g_object_get_data(G_OBJECT(combo), "setting");
  gint value = gtk_combo_box_get_active(combo);
  if (value < 0)
    return;
  *field = value;
  zd_save(d);
}
gboolean zd_config_export(ZdDock *d, const gchar *path, GError **error) {
  if (d->save_blocked) {
    g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_FAILED,
                        _("当前配置不可保存，请先修复配置文件。"));
    return FALSE;
  }
  if (!zd_save(d)) {
    g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_FAILED,
                        _("无法保存当前配置，未创建备份。"));
    return FALSE;
  }
  gchar *contents = NULL;
  gsize size;
  gboolean result =
      g_file_get_contents(d->rc_path, &contents, &size, error) &&
      g_file_set_contents_full(path, contents, size,
                               G_FILE_SET_CONTENTS_CONSISTENT, 0600, error);
  g_free(contents);
  return result;
}
gboolean zd_config_import(ZdDock *d, const gchar *path, GError **error) {
  GStatBuf file_stat;
  if (g_stat(path, &file_stat) || !S_ISREG(file_stat.st_mode) ||
      file_stat.st_size > 1024 * 1024) {
    g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                        _("配置必须是小于 1 MiB 的普通文件。"));
    return FALSE;
  }
  gchar *contents = NULL;
  gsize size;
  if (!g_file_get_contents(path, &contents, &size, error))
    return FALSE;
  GKeyFile *file = g_key_file_new();
  gboolean valid = size <= 1024 * 1024 &&
                   g_key_file_load_from_data(file, contents, size,
                                             G_KEY_FILE_KEEP_COMMENTS, error);
  gsize n = 0;
  GError *pin_error = NULL;
  gchar **pins =
      valid && g_key_file_has_key(file, "Dock", "Pinned", NULL)
          ? g_key_file_get_string_list(file, "Dock", "Pinned", &n, &pin_error)
          : NULL;
  if (pin_error) {
    valid = FALSE;
    g_clear_error(&pin_error);
  }
  gsize rules = 0;
  gchar **rule_keys =
      valid ? g_key_file_get_keys(file, "Associations", &rules, NULL) : NULL;
  if (rules > 512)
    valid = FALSE;
  for (gsize i = 0; valid && rule_keys && i < rules; i++) {
    gchar *value =
        g_key_file_get_string(file, "Associations", rule_keys[i], NULL);
    valid = strlen(rule_keys[i]) == 64 && value && g_path_is_absolute(value) &&
            g_str_has_suffix(value, ".desktop");
    for (guint j = 0; valid && rule_keys[i][j]; j++)
      valid = g_ascii_isxdigit(rule_keys[i][j]);
    g_free(value);
  }
  g_strfreev(rule_keys);
  if (valid) {
    valid = g_key_file_has_group(file, "Dock") && n <= 512;
    for (gsize i = 0; valid && pins && i < n; i++)
      valid =
          g_path_is_absolute(pins[i]) && g_str_has_suffix(pins[i], ".desktop");
  }
  if (!valid) {
    if (!error || !*error)
      g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                          _("所选文件不是有效的 Zero Dock 配置。"));
    g_strfreev(pins);
    g_key_file_unref(file);
    g_free(contents);
    return FALSE;
  }
  gchar *backup = g_strdup_printf("%s.backup-%" G_GINT64_FORMAT, d->rc_path,
                                  g_get_real_time());
  gboolean saved = zd_config_export(d, backup, error);
  g_free(backup);
  if (!saved ||
      !g_file_set_contents_full(d->rc_path, contents, size,
                                G_FILE_SET_CONTENTS_CONSISTENT, 0600, error)) {
    g_strfreev(pins);
    g_key_file_unref(file);
    g_free(contents);
    return FALSE;
  }
  zd_preview_hide(d);
  if (d->menu)
    gtk_widget_destroy(d->menu);
  for (GList *l = d->buttons, *next; l; l = next) {
    next = l->next;
    ZdButton *b = l->data;
    g_clear_object(&b->thumbnail);
    if (!b->pinned)
      continue;
    if (b->window) {
      b->pinned = FALSE;
      g_clear_pointer(&b->desktop, g_free);
      g_free(b->key);
      b->key = g_strdup_printf("window:%lu", xfw_window_x11_get_xid(b->window));
    } else {
      d->buttons = g_list_delete_link(d->buttons, l);
      zd_button_free(b);
    }
  }
  zd_settings_defaults(d);
  zd_settings_load(d, file);
  zd_associations_load(d, file);
  d->app_generation++;
  for (gsize i = 0; pins && i < n; i++)
    zd_add_pin(d, pins[i]);
  zd_refresh(d);
  zd_save(d);
  g_strfreev(pins);
  g_key_file_unref(file);
  g_free(contents);
  return TRUE;
}
static void config_response(GtkDialog *dialog, gint response, ZdDock *d) {
  gboolean restored = FALSE;
  if (response == GTK_RESPONSE_ACCEPT) {
    gchar *path = gtk_file_chooser_get_filename(GTK_FILE_CHOOSER(dialog));
    gboolean restore =
        GPOINTER_TO_INT(g_object_get_data(G_OBJECT(dialog), "restore"));
    GError *error = NULL;
    gboolean result = restore ? zd_config_import(d, path, &error)
                              : zd_config_export(d, path, &error);
    if (!result) {
      zd_show_error(d, error->message);
      g_clear_error(&error);
    }
    restored = restore && result;
    g_free(path);
  }
  gtk_widget_destroy(GTK_WIDGET(dialog));
  if (restored && d->settings) {
    gtk_widget_destroy(d->settings);
    zd_configure(d->plugin, d);
  }
}
static void config_chooser(GtkButton *button, ZdDock *d) {
  gboolean restore =
      GPOINTER_TO_INT(g_object_get_data(G_OBJECT(button), "restore"));
  GtkWidget *dialog = gtk_file_chooser_dialog_new(
      restore ? _("恢复 Zero Dock 配置") : _("备份 Zero Dock 配置"),
      GTK_WINDOW(d->settings),
      restore ? GTK_FILE_CHOOSER_ACTION_OPEN : GTK_FILE_CHOOSER_ACTION_SAVE,
      _("取消"), GTK_RESPONSE_CANCEL, restore ? _("恢复") : _("保存"),
      GTK_RESPONSE_ACCEPT, NULL);
  if (!restore) {
    gtk_file_chooser_set_current_name(GTK_FILE_CHOOSER(dialog),
                                      "zero-dock-backup.rc");
    gtk_file_chooser_set_do_overwrite_confirmation(GTK_FILE_CHOOSER(dialog),
                                                   TRUE);
  }
  GtkWidget *description = gtk_label_new(
      restore ? _("恢复固定应用、手动关联与设置；恢复前自动备份当前配置。")
              : _("备份包含固定启动器和手动关联的本机文件路径。"));
  gtk_file_chooser_set_extra_widget(GTK_FILE_CHOOSER(dialog), description);
  gtk_widget_show(description);
  gtk_window_set_destroy_with_parent(GTK_WINDOW(dialog), TRUE);
  gtk_window_set_skip_taskbar_hint(GTK_WINDOW(dialog), TRUE);
  g_object_set_data(G_OBJECT(dialog), "restore", GINT_TO_POINTER(restore));
  g_signal_connect(dialog, "response", G_CALLBACK(config_response), d);
  gtk_widget_show(dialog);
}
void zd_configure(XfcePanelPlugin *p, ZdDock *d) {
  if (d->settings) {
    gtk_window_present(GTK_WINDOW(d->settings));
    return;
  }
  d->settings = gtk_dialog_new_with_buttons(
      _("Zero Dock 设置"), NULL, GTK_DIALOG_DESTROY_WITH_PARENT, _("关闭"),
      GTK_RESPONSE_CLOSE, NULL);
  gtk_window_set_skip_taskbar_hint(GTK_WINDOW(d->settings), TRUE);
  xfce_panel_plugin_take_window(p, GTK_WINDOW(d->settings));
  GtkWidget *content = gtk_dialog_get_content_area(GTK_DIALOG(d->settings));
  GtkWidget *scroller = gtk_scrolled_window_new(NULL, NULL);
  gtk_scrolled_window_set_policy(GTK_SCROLLED_WINDOW(scroller),
                                 GTK_POLICY_NEVER, GTK_POLICY_AUTOMATIC);
  GtkWidget *box = gtk_box_new(GTK_ORIENTATION_VERTICAL, 12);
  gtk_container_add(GTK_CONTAINER(scroller), box);
  gtk_box_pack_start(GTK_BOX(content), scroller, TRUE, TRUE, 0);
  GdkRectangle area = {0, 0, 1024, 768};
  GdkMonitor *monitor =
      gtk_widget_get_realized(GTK_WIDGET(p))
          ? gdk_display_get_monitor_at_window(
                gdk_display_get_default(), gtk_widget_get_window(GTK_WIDGET(p)))
          : gdk_display_get_primary_monitor(gdk_display_get_default());
  if (monitor)
    gdk_monitor_get_workarea(monitor, &area);
  gtk_window_set_default_size(GTK_WINDOW(d->settings),
                              MIN(560, MAX(200, area.width - 48)),
                              MIN(600, MAX(200, area.height - 48)));
  gtk_container_set_border_width(GTK_CONTAINER(box), 16);
  gtk_box_set_spacing(GTK_BOX(box), 12);
  const gchar *labels[] = {_("显示可点击的窗口预览"), _("同应用多窗口显示序号"),
                           _("显示所有工作区的窗口"), _("滚轮切换窗口")};
  const gchar *keys[] = {"previews", "numbers", "workspaces", "scroll"};
  gboolean vals[] = {d->previews, d->numbers, d->all_workspaces,
                     d->scroll_windows};
  for (guint i = 0; i < G_N_ELEMENTS(keys); i++) {
    GtkWidget *b = gtk_check_button_new_with_label(labels[i]);
    gtk_toggle_button_set_active(GTK_TOGGLE_BUTTON(b), vals[i]);
    g_object_set_data(G_OBJECT(b), "setting", (gpointer)keys[i]);
    g_signal_connect(b, "toggled", G_CALLBACK(settings_changed), d);
    gtk_box_pack_start(GTK_BOX(box), b, FALSE, FALSE, 0);
  }
  const gchar *values[] = {_("预留图标位置（0 为自动宽度）"),
                           _("预览宽度（像素）"),
                           _("悬停延时（毫秒）"),
                           _("预览刷新间隔（毫秒）"),
                           _("启动等待时间（毫秒）"),
                           _("最多显示图标数（含溢出按钮，0 为不限）")};
  guint *fields[] = {&d->slots,          &d->preview_width,
                     &d->preview_delay,  &d->preview_interval,
                     &d->launch_timeout, &d->max_visible};
  const gint lows[] = {0, 180, 100, 200, 2000, 0},
             highs[] = {32, 600, 2000, 2000, 60000, 64};
  for (guint i = 0; i < G_N_ELEMENTS(fields); i++) {
    GtkWidget *row = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8);
    gtk_box_pack_start(GTK_BOX(row), gtk_label_new(values[i]), TRUE, TRUE, 0);
    GtkWidget *spin = gtk_spin_button_new_with_range(
        lows[i], highs[i], (i == 0 || i == 5) ? 1 : 50);
    gtk_spin_button_set_value(GTK_SPIN_BUTTON(spin), *fields[i]);
    g_object_set_data(G_OBJECT(spin), "setting", fields[i]);
    g_signal_connect(spin, "value-changed", G_CALLBACK(value_changed), d);
    gtk_box_pack_end(GTK_BOX(row), spin, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(box), row, FALSE, FALSE, 0);
  }
  const gchar *click_labels[] = {_("左键行为"), _("中键行为")};
  const gchar *left[] = {_("激活 / 最小化"), _("仅激活"), NULL};
  const gchar *middle[] = {_("启动新窗口"), _("关闭窗口"), _("不操作"), NULL};
  guint *click_fields[] = {&d->left_action, &d->middle_action};
  for (guint i = 0; i < 2; i++) {
    GtkWidget *row = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8);
    GtkWidget *combo = gtk_combo_box_text_new();
    const gchar *const *options = i ? middle : left;
    for (guint j = 0; options[j]; j++)
      gtk_combo_box_text_append_text(GTK_COMBO_BOX_TEXT(combo), options[j]);
    gtk_combo_box_set_active(GTK_COMBO_BOX(combo), *click_fields[i]);
    g_object_set_data(G_OBJECT(combo), "setting", click_fields[i]);
    g_signal_connect(combo, "changed", G_CALLBACK(click_changed), d);
    gtk_box_pack_start(GTK_BOX(row), gtk_label_new(click_labels[i]), TRUE, TRUE,
                       0);
    gtk_box_pack_end(GTK_BOX(row), combo, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(box), row, FALSE, FALSE, 0);
  }
  GtkWidget *backups = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8);
  const gchar *backup_labels[] = {_("备份配置…"), _("恢复配置…")};
  for (guint i = 0; i < 2; i++) {
    GtkWidget *button = gtk_button_new_with_label(backup_labels[i]);
    g_object_set_data(G_OBJECT(button), "restore", GINT_TO_POINTER(i));
    g_signal_connect(button, "clicked", G_CALLBACK(config_chooser), d);
    gtk_box_pack_start(GTK_BOX(backups), button, FALSE, FALSE, 0);
  }
  gtk_box_pack_start(GTK_BOX(box), backups, FALSE, FALSE, 0);
  GtkWidget *actions = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8),
            *reset =
                gtk_button_new_with_label(_("恢复默认设置（保留固定应用）")),
            *export = gtk_button_new_with_label(_("导出诊断信息"));
  g_signal_connect(reset, "clicked", G_CALLBACK(reset_settings), d);
  g_signal_connect(export, "clicked", G_CALLBACK(export_diagnostics), d);
  gtk_box_pack_start(GTK_BOX(actions), reset, FALSE, FALSE, 0);
  gtk_box_pack_start(GTK_BOX(actions), export, FALSE, FALSE, 0);
  gtk_box_pack_start(GTK_BOX(box), actions, FALSE, FALSE, 0);
  GtkWidget *info = gtk_label_new(_("固定图标复用首个窗口，其他窗口独立显示。\n"
                                    "拖入 .desktop 文件可固定应用。\n"
                                    "喇叭滚轮调整应用音量，最高 200%。\n"
                                    "预览画面仅保存在内存中。"));
  gtk_label_set_xalign(GTK_LABEL(info), 0);
  gtk_box_pack_start(GTK_BOX(box), info, FALSE, FALSE, 4);
  g_signal_connect_swapped(d->settings, "response",
                           G_CALLBACK(gtk_widget_destroy), d->settings);
  g_signal_connect(d->settings, "destroy", G_CALLBACK(settings_destroy), d);
  gtk_widget_show_all(d->settings);
}
