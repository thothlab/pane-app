/**
 * English translations. Source of truth — keep this file structurally
 * complete; mirror new keys into `ru.ts` (and any future locale).
 * Flatten into dot-notation lookup happens in `index.ts`.
 *
 * NOTE: this file uses plain strings (not `as const`) so the `Dict`
 * type captures the *shape* of translations, not the literal English
 * values. With `as const` other locales (ru.ts) would have to match
 * each English string exactly, which is obviously absurd. The shape is
 * what we enforce — every locale must have every key.
 */
const en = {
  nav: {
    captures: "Captures",
    rules: "Rules",
    devices: "Devices",
    settings: "Settings",
    docs: "Docs",
    about: "About",
    docs_title: "Open documentation in browser",
    filters: "Filters",
    delete_filter: "Delete filter",
    delete_filter_confirm: "Delete filter \"{name}\"?",
    apply_filter: "Apply \"{query}\"",
  },
  proxy: {
    start: "Start proxy",
    stop: "Stop proxy",
    running: "running",
    stopped: "stopped",
  },
  updates: {
    update_to: "Update to v{version}",
    installing: "Installing…",
    install_title: "Install Pane v{version} and restart",
    check_for_updates: "Check for updates",
    checking: "Checking…",
    up_to_date: "You're on the latest version.",
    server_unreachable: "Couldn't reach the update server. Try again later.",
    last_checked: "Last checked: {time}",
  },
  devices: {
    title: "Devices",
    help_title: "USB pairing: iOS / Android setup walkthrough",
    refresh: "Refresh",
    attached_section: "Attached over USB",
    paired_section: "Paired",
    no_attached:
      "No devices detected. Plug in your iPhone or Android, allow trust / USB debugging.",
    no_paired: "No paired devices yet.",
    add: "Add",
    adding: "Adding…",
    resync: "Re-sync",
    resync_title: "Re-apply USB port-forwarding + proxy setup",
    remove: "Remove",
    remove_confirm: "Remove device and revoke setup?",
    boundaries_title: "Use only on devices you own.",
    boundaries_body:
      "Pane is intended for inspecting your own apps and authorized security work. Don't point it at devices or applications you lack permission to inspect.",
    tooling_missing_title: "Android tooling not found",
    almost_there: "Almost there — finish CA install on the device.",
    add_failed: "add failed",
    resync_failed: "re-sync failed",
    manual_install_toggle: "How to install the CA certificate",
    manual_install_intro:
      "Your Android build (most commonly Samsung One UI on Android 16+) blocks programmatic CA installs. Pane has already pushed the certificate to your device — finish the install yourself:",
    manual_install_step1:
      "On the phone, open <strong>Settings → Biometrics & security → Other security settings → Install from device storage → CA certificate</strong>.",
    manual_install_step2: "On the warning screen tap <strong>Install anyway</strong>.",
    manual_install_step3:
      "In the file picker, open <strong>Internal storage → Pane</strong> and pick <code>pane-ca.pem</code>.",
    manual_install_step4: "Enter your screen-lock PIN/pattern when prompted.",
    manual_install_lockscreen_note:
      "Without a lock-screen PIN/pattern, Android refuses user CA installs — set one first if needed. After installation, debug builds with",
    manual_install_lockscreen_note_after:
      "trusting user CAs will accept Pane. Release builds with TLS pinning need extra bypass.",
    copy_path_title: "Copy path",
  },
  settings: {
    title: "Settings",
    appearance_section: "Appearance",
    theme_label: "Theme",
    theme_system: "System",
    theme_light: "Light",
    theme_dark: "Dark",
    language_label: "Language",
  },
  about: {
    title: "About Pane",
    version_label: "Version",
    intro:
      "A modern HTTPS network debugger focused on one thing: <strong>making device setup take 30 seconds instead of 15 minutes.</strong> No certificate trust dance, no Wi-Fi proxy editing — plug your iPhone or Android in over USB and click Add.",
    boundaries_title: "Boundaries",
    boundaries_1:
      "Designed for inspecting <strong>your own</strong> apps and authorized security work.",
    boundaries_2:
      "Doesn't bypass certificate pinning. When pinning blocks inspection, you'll see why.",
    boundaries_3: "Not a production traffic monitor. Not a packet-level capture tool.",
    pinning_title: "Cert pinning",
    pinning_para1:
      "Certificate pinning is a security feature where an app refuses to talk to anyone whose cert doesn't match a pre-baked fingerprint. Our MITM proxy can't impersonate those endpoints — that's by design.",
    pinning_para2:
      "For your own apps, disable pinning in the debug build. For owned-device security research, tools like Frida or Magisk can bypass pinning at runtime; Pane doesn't bundle them.",
    license_title: "License",
    license_body:
      "Apache-2.0. Built on top of rustls, rcgen, libimobiledevice, and the Android Platform Tools.",
  },
  common: {
    cancel: "Cancel",
    save: "Save",
    delete: "Delete",
    edit: "Edit",
    close: "Close",
  },
};

export default en;
export type Dict = typeof en;
