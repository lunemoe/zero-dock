# Contributing

Bug reports, documentation fixes and focused patches are welcome. Explain the problem and observable result; keep unrelated changes separate.

## Local development

Use the dependencies and commands in [README.md](README.md#development). All checks are local; do not add GitHub Actions workflows.

Before submitting behavior changes:

```sh
cargo fmt --manifest-path rust/Cargo.toml --check
meson compile -C build-dev
meson test -C build-dev --print-errorlogs
python tests/run-isolated.py build-dev
```

Use `cargo fmt` for Rust, four-space indentation for Python, and meaningful regression tests for changed behavior. Documentation-only changes do not require desktop integration tests. [Testing](docs/TESTING.md) explains sanitizer coverage and limitations.

## Desktop safety

Run integration tests through the isolation script. Do not terminate or restart an existing XFCE session, panel, compositor or display manager to test a patch. Interactive testing on your desktop is opt-in. Replace only the dock instance through the panel's normal remove/add controls.

## Pull requests

Describe the trigger, resulting behavior, relevant implementation choices and local verification. Mention untested cases. Preserve MIT notices, disclose external code provenance, and submit only code you have permission to contribute. Contributions use the project's MIT license; no additional assignment is requested.

For security problems, use [SECURITY.md](SECURITY.md) instead of a public issue.
