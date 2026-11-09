import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Tauri + Vite for Minewrap v2. The webview talks only to the Tauri IPC
// adapter (`src/ipc/tauri.ts`) or the in-browser mock (`src/ipc/mock.ts`);
// tokens never enter the webview.
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    host: "127.0.0.1",
  },
  build: {
    target: ["es2022"],
  },
});
