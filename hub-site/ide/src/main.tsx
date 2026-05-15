//! Hub IDE bundle entry. Initializes the WASM module, builds a
//! `HubStudioService`, mounts the shared React UI from
//! `packages/studio-ui` against it, and renders.
//!
//! Mounted by VitePress as either an iframe or as the body of
//! `/edit/...` route on the same Cloudflare Pages site.

import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import "@/styles.css";
import { App } from "@/App";
import { TooltipProvider } from "@/components/ui/tooltip";
import { StudioServiceProvider } from "@/lib/services";
import { installStudioService, useStudio } from "@/lib/store";

import init from "forage-wasm";

import { HubStudioService } from "./HubStudioService";

const queryClient = new QueryClient();

async function boot() {
    // wasm-pack's `--target web` output exports a default `init`
    // function that fetches and instantiates the .wasm binary. Must
    // run before any forage-wasm export gets called.
    await init();

    const service = new HubStudioService();
    installStudioService(service);

    // Path-based routing: `/edit/<author>/<slug>` opens that package.
    // The Cloudflare Pages `_redirects` rule rewrites any sub-path
    // under `/edit/` back to `/edit/index.html`; this bundle pulls the
    // path off `window.location` and resolves the (author, slug) pair.
    const after = window.location.pathname.replace(/^\/edit\/?/, "");
    if (after) {
        const [author, slug] = after.split("/").filter(Boolean);
        if (author && slug) {
            try {
                const versionArtifact = await service.getPackageVersion(
                    author,
                    slug,
                    "latest",
                );
                service.setLoaded({ author, slug, version: versionArtifact });
                // Seed the editor session: emulate the workspace-open +
                // file-open flow Studio's sidebar drives.
                await useStudio
                    .getState()
                    .setActiveFilePath(`${slug}/recipe.forage`);
            } catch (e) {
                console.error("failed to load package", e);
            }
        }
    }

    ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
        <React.StrictMode>
            <StudioServiceProvider service={service}>
                <QueryClientProvider client={queryClient}>
                    <TooltipProvider delayDuration={200}>
                        <App />
                    </TooltipProvider>
                </QueryClientProvider>
            </StudioServiceProvider>
        </React.StrictMode>,
    );
}

void boot();
