//! Vitest global setup. Stubs the Tauri IPC bridge so importing
//! `@tauri-apps/api/core` doesn't reach for `window.__TAURI_INTERNALS__`
//! at module-load time — the harness has no Tauri host.
//!
//! Tests that need to assert against backend calls inject a fake
//! StudioService via `installStudioService` and `StudioServiceProvider`
//! rather than reaching for Tauri's IPC bridge directly.

import "@testing-library/jest-dom/vitest";

type TauriInternals = {
    invoke: (cmd: string, args?: unknown) => Promise<unknown>;
    transformCallback: (
        callback?: (response: unknown) => void,
        once?: boolean,
    ) => number;
};

const noop = () => 0;

(globalThis as unknown as { __TAURI_INTERNALS__: TauriInternals }).__TAURI_INTERNALS__ = {
    invoke: async () => undefined,
    transformCallback: noop,
};

// jsdom doesn't ship `matchMedia`; shadcn's `useIsMobile` calls it on
// mount. Stub to a never-matches MediaQueryList so the sidebar renders
// the desktop variant.
if (typeof window !== "undefined" && typeof window.matchMedia !== "function") {
    Object.defineProperty(window, "matchMedia", {
        writable: true,
        value: (query: string): MediaQueryList => ({
            matches: false,
            media: query,
            onchange: null,
            addListener: () => {},
            removeListener: () => {},
            addEventListener: () => {},
            removeEventListener: () => {},
            dispatchEvent: () => false,
        }) as unknown as MediaQueryList,
    });
}

// ResizeObserver / IntersectionObserver are needed by Radix popovers.
class ResizeObserverStub {
    observe() {}
    unobserve() {}
    disconnect() {}
}
if (typeof globalThis.ResizeObserver === "undefined") {
    (globalThis as unknown as { ResizeObserver: typeof ResizeObserver }).ResizeObserver = ResizeObserverStub as unknown as typeof ResizeObserver;
}
