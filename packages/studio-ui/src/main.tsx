import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import "./styles.css";
import { App } from "./App";
import { TooltipProvider } from "@/components/ui/tooltip";
import { StudioServiceProvider, TauriStudioService } from "@/lib/services";
import { installStudioService } from "@/lib/store";

const queryClient = new QueryClient();

// Studio bundle: the active StudioService talks to the Tauri Rust core.
// The hub IDE bundle constructs HubStudioService at its own boot site;
// this file is Studio-specific.
const service = new TauriStudioService();
installStudioService(service, queryClient);

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
