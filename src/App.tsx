import { type Component, lazy } from "solid-js";
import { Route } from "@solidjs/router";
import Layout from "./components/Layout";

const Captures = lazy(() => import("./views/CapturesView"));
const Devices = lazy(() => import("./views/DevicesView"));
const Settings = lazy(() => import("./views/SettingsView"));
const About = lazy(() => import("./views/AboutView"));
const ReplayView = lazy(() => import("./views/ReplayView"));

const App: Component = (props: any) => (
  <Layout>
    <Route path="/" component={Captures} />
    <Route path="/devices" component={Devices} />
    <Route path="/replay/:id" component={ReplayView} />
    <Route path="/settings" component={Settings} />
    <Route path="/about" component={About} />
    {props.children}
  </Layout>
);

export default App;
