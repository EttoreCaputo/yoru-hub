import { mount } from "svelte";
import "@fontsource-variable/bricolage-grotesque";
import "@fontsource-variable/figtree";
import "@fontsource-variable/jetbrains-mono";
import "./app.css";
import App from "./App.svelte";

export default mount(App, { target: document.getElementById("app")! });
