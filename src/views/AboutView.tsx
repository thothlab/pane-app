import { type Component } from "solid-js";

const AboutView: Component = () => (
  <div class="h-full overflow-auto p-6 space-y-6 max-w-3xl">
    <h1 class="text-xl font-semibold">About my-charles</h1>

    <section class="space-y-2 text-sm leading-6">
      <p>
        A modern HTTPS network debugger focused on one thing: <strong>making device setup take
        30 seconds instead of 15 minutes.</strong> No certificate trust dance, no Wi-Fi proxy
        editing — plug your iPhone or Android in over USB and click Add.
      </p>
    </section>

    <section class="space-y-2 text-sm leading-6">
      <h2 class="text-sm font-semibold uppercase tracking-wide text-fg-subtle">Boundaries</h2>
      <ul class="list-disc pl-5 space-y-1 text-fg-subtle">
        <li>Designed for inspecting <strong>your own</strong> apps and authorized security work.</li>
        <li>Doesn't bypass certificate pinning. When pinning blocks inspection, you'll see why.</li>
        <li>Not a production traffic monitor. Not a packet-level capture tool.</li>
      </ul>
    </section>

    <section class="space-y-2 text-sm leading-6">
      <h2 class="text-sm font-semibold uppercase tracking-wide text-fg-subtle">Cert pinning</h2>
      <p>
        Certificate pinning is a security feature where an app refuses to talk to anyone whose
        cert doesn't match a pre-baked fingerprint. Our MITM proxy can't impersonate those
        endpoints — that's by design.
      </p>
      <p>
        For your own apps, disable pinning in the debug build. For owned-device security
        research, tools like Frida or Magisk can bypass pinning at runtime; my-charles doesn't
        bundle them.
      </p>
    </section>

    <section class="space-y-2 text-sm">
      <h2 class="text-sm font-semibold uppercase tracking-wide text-fg-subtle">License</h2>
      <p class="text-fg-subtle">
        Apache-2.0. Built on top of rustls, rcgen, libimobiledevice, and the Android Platform
        Tools.
      </p>
    </section>
  </div>
);

export default AboutView;
