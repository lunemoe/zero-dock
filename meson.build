project('zero-dock', version: '0.3.0', license: 'MIT', meson_version: '>=0.63.0',
        default_options: ['buildtype=debugoptimized'])
cargo = find_program('cargo')
copy = find_program('cp')
python3 = find_program('python3')

# The entire plugin is the Rust crate. Meson runs Cargo exactly once, then
# materializes the plugin module and test host from that shared build output.
rust_sources = files(
  'rust/build.rs',
  'rust/Cargo.toml',
  'rust/Cargo.lock',
  'rust/src/lib.rs',
  'rust/src/plugin_abi.rs',
  'rust/src/apps.rs',
  'rust/src/associations.rs',
  'rust/src/audio.rs',
  'rust/src/button.rs',
  'rust/src/dock.rs',
  'rust/src/ffi_x11.rs',
  'rust/src/ffi_xfce.rs',
  'rust/src/input.rs',
  'rust/src/layout.rs',
  'rust/src/menu.rs',
  'rust/src/preview.rs',
  'rust/src/settings.rs',
  'rust/src/util.rs',
  'rust/src/bin/test_host.rs',
  'rust/src/bin/integration.rs',
)
rust_target_dir = meson.current_build_dir() / 'rust-target'
localedir_abs = get_option('prefix') / get_option('localedir')
cargo_sh = find_program('sh')

rust_build = custom_target('zero-dock-rust-build',
  input: rust_sources,
  output: 'zero-dock-rust-build.stamp',
  build_by_default: true,
  command: [
    cargo_sh, '-c',
    'set -e; out="$(pwd)/$5"; cd "$1/rust"; ZERO_DOCK_VERSION="$2" ZERO_DOCK_LOCALEDIR="$3" cargo build --release --locked --target-dir "$4"; touch "$out"',
    'build', meson.current_source_dir(), meson.project_version(), localedir_abs,
    rust_target_dir, '@OUTPUT@',
  ])

plugin_lib = custom_target('zero-dock-plugin-lib',
  input: rust_build,
  output: 'libzero-dock.so',
  build_by_default: true,
  command: [copy, rust_target_dir / 'release/libzero_dock.so', '@OUTPUT@'],
  install: true,
  install_dir: get_option('libdir') / 'xfce4/panel/plugins')

test_host = custom_target('zero-dock-test-host-bin',
  input: rust_build,
  output: 'zero-dock-test-host',
  build_by_default: true,
  command: [copy, rust_target_dir / 'release/zero-dock-test-host', '@OUTPUT@'])

install_data('data/zero-dock.desktop', install_dir: get_option('datadir') / 'xfce4/panel/plugins')
doc_dir = get_option('datadir') / 'doc/xfce4-zero-dock-plugin'
install_data(['README.md', 'README.zh-CN.md', 'VERIFICATION.md', 'CHANGELOG.md',
              'CONTRIBUTING.md', 'SECURITY.md', 'CODE_OF_CONDUCT.md', 'LICENSE'],
             install_dir: doc_dir)
install_subdir('docs', install_dir: doc_dir)
install_data('LICENSE', install_dir: get_option('datadir') / 'licenses/xfce4-zero-dock-plugin')

if get_option('tests')
  test('rust-unit', cargo,
       args: ['test', '--release', '--locked', '--manifest-path',
              meson.current_source_dir() / 'rust/Cargo.toml',
              '--target-dir', rust_target_dir],
       suite: 'unit')

  xvfb = find_program('Xvfb', required: false)
  xfwm4 = find_program('xfwm4', required: false)
  dbus_run_session = find_program('dbus-run-session', required: false)
  if xvfb.found() and xfwm4.found() and dbus_run_session.found()
    test('isolated-host-smoke', python3,
         args: [meson.current_source_dir() / 'tests/run-isolated.py',
                meson.current_build_dir(), '--iterations', '3', '--seconds', '2'],
         depends: test_host,
         suite: 'integration',
         timeout: 60,
         is_parallel: false)
  endif

  integration_bin = custom_target('zero-dock-integration-bin',
    input: rust_sources,
    output: 'zero-dock-integration',
    command: [
      cargo_sh, '-c',
      'set -e; cd "$1/rust"; ZERO_DOCK_VERSION="$2" ZERO_DOCK_LOCALEDIR="$3" cargo build --release --locked --features test-harness --bin zero-dock-integration --target-dir "$4"; cp "$4/release/zero-dock-integration" "$5"',
      'integration', meson.current_source_dir(), meson.project_version(), localedir_abs,
      rust_target_dir, meson.current_build_dir() / 'zero-dock-integration',
    ])

  if xvfb.found() and xfwm4.found() and dbus_run_session.found()
    test('rust-integration-core', python3,
         args: [meson.current_source_dir() / 'tests/run-integration.py',
                meson.current_build_dir(), '--suite', 'core'],
         depends: integration_bin,
         suite: ['integration', 'behavior'],
         timeout: 180,
         is_parallel: false)

    test('rust-integration-improvements-scale2', python3,
         args: [meson.current_source_dir() / 'tests/run-integration.py',
                meson.current_build_dir(), '--scenario', 'improvements',
                '--scale', '2', '--geometry', '2048x1536'],
         depends: integration_bin,
         suite: ['integration', 'hidpi'],
         timeout: 120,
         is_parallel: false)

    test('rust-integration-stress', python3,
         args: [meson.current_source_dir() / 'tests/run-integration.py',
                meson.current_build_dir(), '--suite', 'stress',
                '--stress-cycles', '5'],
         depends: integration_bin,
         suite: ['integration', 'stress'],
         timeout: 180,
         is_parallel: false)

    wrapper = find_program('/usr/lib/xfce4/panel/wrapper-2.0', required: false)
    if wrapper.found()
      test('rust-integration-external-wrapper', python3,
           args: [meson.current_source_dir() / 'tests/run-integration.py',
                  meson.current_build_dir(), '--scenario', 'external-wrappers'],
           depends: [integration_bin, plugin_lib],
           suite: ['integration', 'wrapper'],
           timeout: 90,
         is_parallel: false)
    endif

    pipewire = find_program('pipewire', required: false)
    pipewire_pulse = find_program('pipewire-pulse', required: false)
    wireplumber = find_program('wireplumber', required: false)
    pactl = find_program('pactl', required: false)
    pacat = find_program('pacat', required: false)
    if pipewire.found() and pipewire_pulse.found() and wireplumber.found() and pactl.found() and pacat.found()
      test('rust-integration-audio-recovery', python3,
           args: [meson.current_source_dir() / 'tests/run-integration.py',
                  meson.current_build_dir(), '--suite', 'audio', '--private-audio'],
           depends: integration_bin,
           suite: ['integration', 'audio'],
           timeout: 90,
         is_parallel: false)
    endif
  endif

  alias_target('test-host', test_host)
  alias_target('integration-runner', integration_bin)
endif
subdir('po')
