import { mount } from "svelte";
import "@fontsource-variable/jetbrains-mono";
import "./app.css";
import App from "./App.svelte";
import Notch from "./Notch.svelte";

// The same page serves the window in the notch: the app marks it, and
// `?notch` shows it in the browser preview.
const notch = !!(window as { __YORU_NOTCH__?: boolean }).__YORU_NOTCH__ || new URLSearchParams(location.search).has("notch");
if (notch) document.documentElement.classList.add("notch");
// Scrollbars show only while something scrolls: each scrolled element is
// marked for a moment after its last scroll (app.css draws the thumb then).
const quiet = new WeakMap<Element, ReturnType<typeof setTimeout>>();
addEventListener(
  "scroll",
  (e) => {
    const el = e.target instanceof Element ? e.target : document.documentElement;
    el.setAttribute("data-scrolling", "");
    clearTimeout(quiet.get(el));
    quiet.set(el, setTimeout(() => el.removeAttribute("data-scrolling"), 900));
  },
  { capture: true, passive: true },
);

export default mount(notch ? Notch : App, { target: document.getElementById("app")! });
