project('zero-dock', 'c', version: '0.3.0', license: 'MIT', meson_version: '>=0.63.0',
        default_options: ['c_std=c11', 'warning_level=2', 'buildtype=debugoptimized'])
cc = meson.get_compiler('c')
config = configuration_data()
config.set_quoted('ZERO_DOCK_VERSION', meson.project_version())
config.set_quoted('GETTEXT_PACKAGE', 'zero-dock')
config.set_quoted('LOCALEDIR', get_option('prefix') / get_option('localedir'))
configure_file(output: 'config.h', configuration: config)
inc = include_directories('.')
deps = [dependency('libxfce4panel-2.0', version: '>=4.20'), dependency('libxfce4ui-2', version: '>=4.18'),
        dependency('libxfce4windowing-0', version: '>=4.20'), dependency('gtk+-3.0', version: '>=3.24'), dependency('gio-unix-2.0'),
        dependency('libpulse-mainloop-glib'), dependency('xcomposite'), dependency('x11'), dependency('xi'),
        cc.find_library('m')]
core = static_library('zero-dock-core', ['src/plugin.c', 'src/apps.c', 'src/settings.c', 'src/associations.c', 'src/layout.c', 'src/buttons.c', 'src/menu.c', 'src/preview.c', 'src/audio.c', 'src/input.c'],
                      dependencies: deps, include_directories: inc, pic: true)
shared_module('zero-dock', 'src/entry.c', link_with: core, dependencies: deps,
              include_directories: inc,
              install: true, install_dir: get_option('libdir') / 'xfce4/panel/plugins')
install_data('data/zero-dock.desktop', install_dir: get_option('datadir') / 'xfce4/panel/plugins')
doc_dir = get_option('datadir') / 'doc/xfce4-zero-dock-plugin'
install_data(['README.md', 'README.zh-CN.md', 'VERIFICATION.md', 'CHANGELOG.md',
              'CONTRIBUTING.md', 'SECURITY.md', 'CODE_OF_CONDUCT.md', 'LICENSE'],
             install_dir: doc_dir)
install_subdir('docs', install_dir: doc_dir)
install_data('LICENSE', install_dir: get_option('datadir') / 'licenses/xfce4-zero-dock-plugin')
if get_option('tests')
  executable('zero-dock-test-host', 'tests/host.c', link_with: core, dependencies: deps, include_directories: inc)
  model_tests = executable('zero-dock-tests', 'tests/test-core.c', link_with: core, dependencies: deps, include_directories: inc, c_args: ['-DTEST_DESKTOP="' + meson.current_source_dir() / 'data/zero-dock.desktop' + '"'])
  test('core', model_tests)
  executable('zero-dock-integration', 'tests/integration.c', link_with: core, dependencies: deps + [dependency('xtst')], include_directories: inc)
  executable('zero-dock-inspect-apps', 'tests/inspect-apps.c', link_with: core, dependencies: deps, include_directories: inc)
  executable('zero-dock-real-preview', 'tests/real-preview.c', link_with: core, dependencies: deps, include_directories: inc)
endif
subdir('po')
