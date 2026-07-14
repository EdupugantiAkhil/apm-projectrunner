# Switchyard Tab Route extension

This unpacked Chromium Manifest V3 extension connects the active tab to one declared
Switchyard route. It has no build step and no third-party dependencies.

Before loading it, edit [`routes.js`](routes.js) and `host_permissions` in
[`manifest.json`](manifest.json). Keep only route identifiers from the active deployment
and the same exact local HTTP endpoint patterns in both files. The service worker
rejects non-local endpoint patterns.

1. Open `chrome://extensions` (or the equivalent page in Edge).
2. Enable **Developer mode**.
3. Choose **Load unpacked** and select this directory.
4. Pin **Switchyard Tab Route**, open a UI tab, and choose one route.

The rule belongs to that tab only. **Disconnect** removes it immediately, and closing
the tab removes its session rule. Disable the extension from the extensions page to
pause all rules. Choose **Remove** there to uninstall it; no application or deployment
files are changed.

See [Browser route identity](../../docs/browser-routing.md) for the header trust
boundary, permission scope, managed-profile fallback, and cleanup instructions.
