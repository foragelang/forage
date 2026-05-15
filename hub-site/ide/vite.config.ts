import path from "node:path";
import react from "@vitejs/plugin-react";
import tailwind from "@tailwindcss/vite";
import wasm from "vite-plugin-wasm";
import { defineConfig } from "vitest/config";

// The hub IDE shares its components with Studio via path aliases into
// `packages/studio-ui/src`. Both Vite and tsc need to resolve `@/`
// against that source tree at build time; tsconfig.json has the
// matching `paths` entry.
//
// VitePress serves the bundle as a static asset at `/edit/`, so the
// built output goes into `dist/` with a `base` of `/edit/` so any
// asset URLs in the produced HTML resolve correctly when mounted on
// the hub site.
const STUDIO_UI_SRC = path.resolve(__dirname, "../../packages/studio-ui/src");

export default defineConfig({
    base: "/edit/",
    plugins: [react(), tailwind(), wasm()],
    resolve: {
        alias: {
            "@": STUDIO_UI_SRC,
        },
    },
    clearScreen: false,
    server: {
        port: 5174,
        strictPort: true,
    },
    build: {
        outDir: "dist",
    },
    test: {
        environment: "jsdom",
        setupFiles: ["./src/test-setup.ts"],
    },
});
