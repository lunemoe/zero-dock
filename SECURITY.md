# Security policy

Security fixes target the latest published release and the main branch. Older releases do not have a separate maintenance guarantee.

Use GitHub's **Security → Report a vulnerability** for a private report:
https://github.com/lunemoe/zero-dock/security/advisories/new

Include affected versions, reproduction steps, impact and a minimal example. Do not publish sensitive data in an issue. There is no guaranteed response time.

Zero Dock runs as the logged-in desktop user and needs no elevated runtime privileges. Dropped `.desktop` files are launchers: pin and launch only files you trust. Previews remain in memory; the plugin has no network or telemetry component. XInput2 listening is limited to pointer events; volume actions apply only over this instance's speaker controls. Linux `/proc` metadata is used for audio matching.
