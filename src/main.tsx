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

const root = document.getElementById("root");
if (!root) throw new Error("#root not found");

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
