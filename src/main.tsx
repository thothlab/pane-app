/* @refresh reload */
import { render } from "solid-js/web";
import { lazy } from "solid-js";
import { Route, Router } from "@solidjs/router";
import Layout from "./components/Layout";
import "./styles/index.css";

const Captures = lazy(() => import("./views/CapturesView"));
const Devices = lazy(() => import("./views/DevicesView"));
const Settings = lazy(() => import("./views/SettingsView"));
const About = lazy(() => import("./views/AboutView"));
const ReplayView = lazy(() => import("./views/ReplayView"));

const root = document.getElementById("root");
if (!root) throw new Error("#root not found");

render(
  () => (
    <Router root={Layout}>
      <Route path="/" component={Captures} />
      <Route path="/devices" component={Devices} />
      <Route path="/replay/:id" component={ReplayView} />
      <Route path="/settings" component={Settings} />
      <Route path="/about" component={About} />
    </Router>
  ),
  root,
);
