# Local release procedure

Release preparation is manual. No GitHub Actions, CI workflow or automated publishing runs are configured.

1. Update the Meson version, `packaging/PKGBUILD`, changelog and verified support scope.
2. Run local build, unit, integration, formatting and sanitizer checks in [TESTING.md](TESTING.md).
3. Review the intended source changes. The archive includes tracked files and new source/documentation/catalog files in the project's source directories; generated builds, evidence and personal configuration must stay excluded.
4. Run `python tools/make-dist.py`. It writes a deterministic source archive under `dist/`, copies it to `packaging/`, and prints its SHA-256. Packaging is excluded from the archive to avoid a self-referential checksum. Inspect the archive's file list before packaging.
5. Record that digest in `packaging/PKGBUILD`. Run `makepkg --cleanbuild --force` in `packaging/` and inspect the result. Source releases are portable; locally compiled Arch packages may use host-specific compiler settings and are not automatically published.
6. Commit, tag `v<version>`, push, and create a GitHub release with the source archive, `SHA256SUMS` and matching `PKGBUILD`. Use prerelease status until the release is ready for a stable designation.
7. Download the published source asset and verify its digest. Extract and build it afresh to check the actual distributed source.

Keep release notes factual. Include tested environments and unresolved compatibility limits. Do not restart a user's desktop while validating a release.

After validation and installation, `python tools/clean.py --apply` removes generated build/evidence/packaging files. It preserves tracked content and `dist/` release artifacts. Keep only the desired release version in `dist/`.
