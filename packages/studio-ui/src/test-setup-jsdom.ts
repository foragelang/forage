//! Shared jsdom shims for the React test surfaces. Imported from both
//! `packages/studio-ui/src/test-setup.ts` (Studio) and
//! `hub-site/ide/src/test-setup.ts` (hub IDE) so the two surfaces don't
//! drift on which browser APIs jsdom is missing.
//!
//! Stubs:
//! - `window.matchMedia` — used by shadcn's `useIsMobile` to pick the
//!   desktop sidebar variant. The stub always reports `matches: false`.
//! - `globalThis.ResizeObserver` — Radix popovers and Monaco want it on
//!   import. The stub no-ops every method.

import "@testing-library/jest-dom/vitest";

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

class ResizeObserverStub {
    observe() {}
    unobserve() {}
    disconnect() {}
}
if (typeof globalThis.ResizeObserver === "undefined") {
    (globalThis as unknown as { ResizeObserver: typeof ResizeObserver }).ResizeObserver =
        ResizeObserverStub as unknown as typeof ResizeObserver;
}
