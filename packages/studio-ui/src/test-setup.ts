//! Vitest global setup for Studio. Stubs the Tauri IPC bridge so
//! importing `@tauri-apps/api/core` doesn't reach for
//! `window.__TAURI_INTERNALS__` at module-load time — the harness has
//! no Tauri host. jsdom browser-API shims live in `test-setup-jsdom.ts`
//! and are shared with the hub IDE.
//!
//! Tests that need to assert against backend calls inject a fake
//! StudioService via `installStudioService` and `StudioServiceProvider`
//! rather than reaching for Tauri's IPC bridge directly.

import "./test-setup-jsdom";

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
