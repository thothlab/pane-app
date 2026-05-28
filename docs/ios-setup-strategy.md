# iOS USB setup strategy (Plan A / Plan B)

Direct USB pairing on modern iOS is fragile. We ship two paths and choose
automatically.

## Plan A — Profile push via lockdownd (preferred, iOS 16)

1. `idevicepair -u <udid> pair` → user taps Trust on the device.
2. Build a `.mobileconfig` containing our root CA payload **and** a global HTTP
   proxy payload pointing at `127.0.0.1:8888`.
3. Push the profile via `ideviceinstaller` / direct lockdownd `com.apple.misagent`
   service.
4. Walk the user through: Settings → General → VPN & Device Management →
   tap profile → Install. Then: Settings → General → About → Certificate
   Trust Settings → enable for our root.
5. Start `iproxy 8888 8888 -u <udid>` so the device-side `127.0.0.1` reaches
   the desktop proxy.

**Works on:** iOS 16, partially iOS 17.

## Plan B — QR fallback (iOS 17+ where Plan A is blocked)

1. App opens an HTTP server on a random port bound to the LAN IP (token-protected).
2. Renders a QR pointing to `http://<lan>:<port>/setup?t=<token>`.
3. User scans → Safari opens → tap "Install profile" → same trust steps as Plan A.
4. Desktop proxy is reachable via the LAN IP encoded in the profile's proxy payload.
5. The setup server self-terminates on success or after 15 minutes.

**Works on:** iOS 16/17/18, requires user and device on the same Wi-Fi.

## Decision logic

The device-add flow attempts Plan A. If `ideviceinstaller` is missing or the
push fails, we surface a "Switch to QR setup" CTA and start the QR server.
Both paths converge on the same `Device.state=ready` once a TLS handshake
through the proxy succeeds.

## Tested matrix (target — to be validated on real hardware)

| iOS | Plan A | Plan B |
| --- | ------ | ------ |
| 16  | ✓      | ✓      |
| 17  | partial | ✓      |
| 18  | ✗      | ✓      |
