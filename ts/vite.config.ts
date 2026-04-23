import { defineConfig } from "vite";

// Top-level `await init()` for both WASM engines needs ES2022-class
// output. Defaulting `target: "esnext"` keeps the build simple and
// matches the realistic audience for a Korean-office-doc preview
// demo (modern desktop browsers).
export default defineConfig({
  build: {
    target: "esnext",
  },
  optimizeDeps: {
    esbuildOptions: { target: "esnext" },
  },
});
