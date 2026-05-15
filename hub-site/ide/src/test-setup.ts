//! Vitest setup for the hub IDE. Delegates to the shared
//! `packages/studio-ui/src/test-setup-jsdom.ts` for the jsdom browser
//! shims; the hub IDE doesn't need Studio's Tauri-internals stub, so
//! that bit stays in Studio's setup.

import "@/test-setup-jsdom";
