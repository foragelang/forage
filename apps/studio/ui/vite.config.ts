import path from "node:path";
import react from "@vitejs/plugin-react";
import tailwind from "@tailwindcss/vite";
import { defineConfig } from "vitest/config";

export default defineConfig({
    plugins: [react(), tailwind()],
    resolve: {
        alias: {
            "@": path.resolve(__dirname, "./src"),
        },
    },
    clearScreen: false,
    server: {
        port: 5173,
        strictPort: true,
    },
    build: {
        outDir: "dist",
    },
    test: {
        // jsdom for component tests that mount React into a DOM. Pure
        // store-logic tests don't need it but the jsdom environment is
        // cheap to set up globally.
        environment: "jsdom",
        // Tauri's plugin entrypoints check for `__TAURI_INTERNALS__` on
        // the window. We don't exercise them under jsdom, but
        // importing the plugin module shouldn't blow up.
        setupFiles: ["./src/test-setup.ts"],
    },
});
