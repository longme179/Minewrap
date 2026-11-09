import "./styles/tokens.css";
import { mount } from "svelte";
import App from "./App.svelte";

// IPC adapter selection lives in `src/ipc/factory.ts` — Tauri in the
// desktop webview, mock in a plain browser (Vite). App.svelte creates
// its own adapter via `createAdapter()`; main.ts just mounts.
const app = mount(App, { target: document.getElementById("app")! });
export default app;
