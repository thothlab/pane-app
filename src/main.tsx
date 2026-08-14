/* @refresh reload */
import { render } from "solid-js/web";
import { ErrorBoundary, lazy, Suspense, type ParentComponent } from "solid-js";
import { Route, Router } from "@solidjs/router";
import Layout from "./components/Layout";
import "./styles/index.css";
import "./stores/theme";
import "./stores/font-scale";

const Captures = lazy(() => import("./views/CapturesView"));
const Devices = lazy(() => import("./views/DevicesView"));
const Settings = lazy(() => import("./views/SettingsView"));
const About = lazy(() => import("./views/AboutView"));
const ReplayView = lazy(() => import("./views/ReplayView"));
const Rules = lazy(() => import("./views/RulesView"));
const LogcatView = lazy(() => import("./views/LogcatView"));

// Every route is `lazy()`. Without a boundary, a route module that fails to
// load — or a component that throws on first render — renders *nothing*: the
// sidebar stays, the content pane goes white, and there is no message
// anywhere. That failure mode cost a full debugging session, so it is now
// impossible: the boundary prints the error, and the Suspense fallback proves
// the difference between "still loading" and "died".
const RouteError: ParentComponent<{ err: unknown; reset: () => void }> = (
  props,
) => (
  <div class="p-6 space-y-3 overflow-auto h-full">
    <div class="text-danger font-medium text-sm">
      This screen failed to load.
    </div>
    <pre class="text-xs whitespace-pre-wrap bg-bg-muted rounded p-3 text-fg">
      {(props.err as Error)?.stack ??
        (props.err as { message?: string })?.message ??
        String(props.err)}
    </pre>
    <button
      class="text-sm px-3 py-1.5 rounded bg-accent text-white hover:opacity-90"
      onClick={() => props.reset()}
    >
      Retry
    </button>
  </div>
);

const Shell: ParentComponent = (props) => (
  <Layout>
    <ErrorBoundary
      fallback={(err, reset) => <RouteError err={err} reset={reset} />}
    >
      <Suspense
        fallback={
          <div class="p-6 text-sm text-fg-muted">Loading this screen…</div>
        }
      >
        {props.children}
      </Suspense>
    </ErrorBoundary>
  </Layout>
);

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
      <Router root={Shell}>
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
