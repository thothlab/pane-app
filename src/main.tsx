/* @refresh reload */
import { render } from "solid-js/web";
import { lazy } from "solid-js";
import { Route, Router } from "@solidjs/router";
import Layout from "./components/Layout";
import "./styles/index.css";
import "./stores/theme";

const Captures = lazy(() => import("./views/CapturesView"));
const Devices = lazy(() => import("./views/DevicesView"));
const Settings = lazy(() => import("./views/SettingsView"));
const About = lazy(() => import("./views/AboutView"));
const ReplayView = lazy(() => import("./views/ReplayView"));
const Rules = lazy(() => import("./views/RulesView"));
const LogcatView = lazy(() => import("./views/LogcatView"));

const root = document.getElementById("root");
if (!root) throw new Error("#root not found");

// Multi-window dispatch. Logcat windows open with
// `index.html?logcat=1&serial=...` — same vite-bundled SPA, but we
// mount a different view so the main `Router` + `Layout` (sidebar,
// proxy controls, captures auto-refresh, etc.) don't load in the
// logcat window. Separate vite entry would mean two bundles for one
// trivial alt-view — not worth the build complexity.
const params = new URLSearchParams(window.location.search);
const isLogcatWindow = params.get("logcat") === "1";

if (isLogcatWindow) {
  render(() => <LogcatView />, root);
} else {
  render(
    () => (
      <Router root={Layout}>
        <Route path="/" component={Captures} />
        <Route path="/devices" component={Devices} />
        <Route path="/rules" component={Rules} />
        <Route path="/replay/:id" component={ReplayView} />
        <Route path="/settings" component={Settings} />
        <Route path="/about" component={About} />
      </Router>
    ),
    root,
  );
}
