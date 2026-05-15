//! React context that injects the active `StudioService` at mount time.
//! Studio's `main.tsx` wraps the tree in `TauriStudioService`; the hub
//! IDE's `main.tsx` wraps it in `HubStudioService`. Components consume
//! the active service via `useStudioService()` and never know which
//! one's behind it.

import { createContext, useContext, type ReactNode } from "react";

import type { StudioService } from "./StudioService";

const StudioServiceContext = createContext<StudioService | null>(null);

export function StudioServiceProvider({
    service,
    children,
}: {
    service: StudioService;
    children: ReactNode;
}) {
    return (
        <StudioServiceContext.Provider value={service}>
            {children}
        </StudioServiceContext.Provider>
    );
}

export function useStudioService(): StudioService {
    const service = useContext(StudioServiceContext);
    if (!service) {
        throw new Error(
            "useStudioService called outside <StudioServiceProvider>. Mount the app with a service.",
        );
    }
    return service;
}
