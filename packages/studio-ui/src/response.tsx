//! Pop-out Response viewer entry point. Separate Vite entry (and
//! Tauri window) from the main editor — the window subscribes to
//! the same Tauri events the main UI does, but mounts only the
//! Response column. Reusing the main React tree would also work but
//! drags in every editor / inspector dependency for what's a tiny
//! sub-component.

import React from "react";
import ReactDOM from "react-dom/client";

import "./styles.css";
import { ResponseWindow } from "@/components/ResponseWindow";
import { TooltipProvider } from "@/components/ui/tooltip";
import { StudioServiceProvider, TauriStudioService } from "@/lib/services";

const service = new TauriStudioService();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
        <StudioServiceProvider service={service}>
            <TooltipProvider delayDuration={200}>
                <ResponseWindow />
            </TooltipProvider>
        </StudioServiceProvider>
    </React.StrictMode>,
);
