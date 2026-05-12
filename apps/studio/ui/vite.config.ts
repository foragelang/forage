import path from "node:path";
import react from "@vitejs/plugin-react";
import tailwind from "@tailwindcss/vite";
import { defineConfig } from "vite";

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
});
