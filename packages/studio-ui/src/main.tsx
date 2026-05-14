import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import "./styles.css";
import { App } from "./App";
import { TooltipProvider } from "@/components/ui/tooltip";

const queryClient = new QueryClient();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
        <QueryClientProvider client={queryClient}>
            <TooltipProvider delayDuration={200}>
                <App />
            </TooltipProvider>
        </QueryClientProvider>
    </React.StrictMode>,
);
