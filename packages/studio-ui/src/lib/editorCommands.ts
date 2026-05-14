//! Imperative commands the editor pane receives from other surfaces
//! (Inspector diagnostic cards, debug pane, etc.).
//!
//! State-as-command is an anti-pattern (see reactive-ui.md): commands
//! are one-shot — observing them through a store field would require
//! edge-triggered guards and reset hacks. Pub/sub keeps the editor's
//! reactive surface lean and lets new emitters self-wire without the
//! store knowing about them.
//!
//! Channel: `forage:reveal-line` — payload is `{ line: number }` where
//! line is 1-based, matching Monaco. EditorPane subscribes via
//! `window.addEventListener`.

const CHANNEL = "forage:reveal-line";

export type RevealLineDetail = { line: number };

export function emitRevealLine(line: number) {
    window.dispatchEvent(
        new CustomEvent<RevealLineDetail>(CHANNEL, { detail: { line } }),
    );
}

export function onRevealLine(handler: (line: number) => void): () => void {
    const listener = (e: Event) => {
        const detail = (e as CustomEvent<RevealLineDetail>).detail;
        if (detail && typeof detail.line === "number") {
            handler(detail.line);
        }
    };
    window.addEventListener(CHANNEL, listener);
    return () => window.removeEventListener(CHANNEL, listener);
}
