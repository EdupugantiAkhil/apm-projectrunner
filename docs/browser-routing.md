# Browser route identity

Switchyard can identify unchanged browser applications in three ways, in strict order:
an explicit per-tab header, the request `Origin`, then a dedicated managed-profile
proxy listener. A request that has no valid identity is rejected rather than sent to an
arbitrary backend.

## Explicit tab header

The extension sends this request header:

```text
X-Switchyard-Route: <route-id>
```

`<route-id>` is a declared identifier of 1–128 lowercase ASCII letters, digits, `.`,
`_`, `:`, or `-`; it starts with a letter or digit. The gateway accepts the value only
when the route exists and the request destination is one of that route's declared local
endpoints. It strips the header before provider delivery by default. Preservation requires
both `spec.identity.stripBeforeForwarding: false` and
`receiveIdentityHeader: true` on the selected provider. The router-wide setting is a
master stripping gate; disabling it alone never exposes the header to providers that have
not explicitly opted in.

The header is routing authority, not user authentication. A gateway must trust it only
from the local extension boundary; LAN clients and arbitrary inbound requests cannot
self-assign routes. Conflicting explicit-header and Origin identities fail closed under
the configured trust policy.

The extension lives at [`extensions/switchyard-route`](../extensions/switchyard-route/).
Its checked-in `routes.js` and `manifest.json` host permission are examples. Replace
them with the active deployment's route IDs and the same exact localhost URL prefixes
before loading the unpacked extension. The
extension then creates one session-only declarative request rule for the active tab.
It never matches remote internet hosts, and it cannot add the header outside the local
host permissions listed in `manifest.json`.

The popup is fully keyboard operable: open it, tab to a declared route, and press Enter
or Space. **Disconnect** removes the current tab rule. Closing a tab also removes its
rule. To pause or remove the extension, use `chrome://extensions`; removal leaves
Switchyard deployments and source trees untouched.

## Managed Chromium profile

When neither the extension nor `Origin` can provide identity, start a dedicated profile:

```sh
switchyard open deployment.yaml ui-1
```

The applied host-gateway plan owns a loopback listener and writes:

```text
.switchyard/generated/<deployment>/managed-profiles/<ui>.json
```

The file identifies its deployment and UI, route, loopback proxy address, and start URL.
It contains no proxy credential or other secret. The host gateway creates a separate
owner-only credential under `.switchyard/run/<deployment>/managed-profiles/`.
`switchyard open` validates those
ownership fields, refuses non-loopback proxy addresses, verifies that the listener is
running, and then launches Chromium with a deployment-scoped directory:

```text
.switchyard/profiles/<deployment>/<ui>/
```

The browser receives both `--proxy-server=http://127.0.0.1:<port>` and
`--proxy-bypass-list=<-loopback>`. The latter deliberately disables Chromium's implicit
localhost bypass so unchanged calls to localhost traverse the selected proxy listener.
The credential is copied only into a private authentication helper inside that profile;
it is never placed in generated metadata, command output, or browser arguments. The
helper is an MV3 extension that answers proxy-only authentication challenges through
[`onAuthRequired`](https://developer.chrome.com/docs/extensions/reference/api/webRequest).

Managed-profile proxying is HTTP-only in this phase and does not implement HTTPS
`CONNECT` or local TLS interception. An HTTPS `startUrl` is rejected with guidance to
use extension-header or Origin routing instead.
The launcher auto-detects Chromium and `chromium-browser`. Set `SWITCHYARD_CHROMIUM` to
an executable path for another Chromium build or Chrome for Testing. [Chrome 137 and
newer removed the required unpacked-extension launch
flag](https://developer.chrome.com/blog/extension-news-june-2025) from branded Chrome, and Edge
does not provide a supported fallback, so neither is auto-detected. Unsupported browsers
fail with an actionable error and are not launched with a partially configured profile.

Close every window using the profile before removing it. Profiles are disposable and
can be deleted explicitly:

```sh
rm -rf .switchyard/profiles/<deployment>/<ui>
```

Deleting a managed browser profile does not delete Docker volumes or source worktrees.
