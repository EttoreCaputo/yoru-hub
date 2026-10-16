import { defineConfig, searchForWorkspaceRoot } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Tauri expects a fixed port and wants its own Rust errors to stay visible.
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
    // Each board's version.txt and changelog live in the repository's firmware/<kind>/.
    fs: { allow: [searchForWorkspaceRoot(process.cwd()), "../firmware"] },
  },
  build: { target: "es2022" },
});
