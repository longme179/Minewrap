import "./styles/tokens.css";
import { mount } from "svelte";
import App from "./App.svelte";
import { MockIpcAdapter } from "./ipc/mock.js";
import { createConsoleStore } from "./stores/console.js";

// In the Tauri webview the real adapter is used; in a plain browser
// (Vite) we fall back to the mock so the UI is usable for screenshots.
const isTauri =
  typeof window !== "undefined" &&
  "__TAURI_INTERNALS__" in window;

if (!isTauri) {
  const adapter = new MockIpcAdapter();
  const store = createConsoleStore();
  void adapter.getLogs("local", "survival", 12).then((lines) => store.append(lines));
  store.attach(adapter);
}

const app = mount(App, { target: document.getElementById("app")! });
export default app;
