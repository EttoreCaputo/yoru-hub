import { mount } from "svelte";
import "@fontsource-variable/bricolage-grotesque";
import "@fontsource-variable/figtree";
import "@fontsource-variable/jetbrains-mono";
import "./app.css";
import App from "./App.svelte";
import Notch from "./Notch.svelte";

// The same page serves the window in the notch: the app marks it, and
// `?notch` shows it in the browser preview.
const notch = !!(window as { __YORU_NOTCH__?: boolean }).__YORU_NOTCH__ || new URLSearchParams(location.search).has("notch");
if (notch) document.documentElement.classList.add("notch");
export default mount(notch ? Notch : App, { target: document.getElementById("app")! });
