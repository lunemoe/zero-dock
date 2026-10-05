i18n = import('i18n')
# The Rust core marks translatable strings with its own t() helper; the glib
# preset only knows _/N_/C_, which would silently extract nothing.
i18n.gettext('zero-dock',
  preset: 'glib',
  args: [
    '--keyword=t',
    '--package-version=' + meson.project_version(),
    '--msgid-bugs-address=https://github.com/lunemoe/zero-dock/issues',
  ])
