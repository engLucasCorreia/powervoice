import { mount } from "svelte";
import App from "./App.svelte";
import "./lib/theme/tokens.css";

const target = document.getElementById("app");
if (!target) {
  throw new Error("missing #app root element");
}

mount(App, { target });
