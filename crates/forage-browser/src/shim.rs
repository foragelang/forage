//! Embedded JS shim injected into every browser-recipe page.
//!
//! Intercepts `fetch` and `XMLHttpRequest.send` so every response body
//! flows back to the host (Studio) via Tauri's event channel. Settle
//! detection is host-side: the host resets an idle timer on every
//! incoming capture and considers the page settled when no captures
//! arrive for the recipe's `noProgressFor` window.

/// The shim source. Embedded at compile time via `include_str!`.
pub const FETCH_INTERCEPT_JS: &str = r#"
(() => {
    if (window.__forage_shim_installed) return;
    window.__forage_shim_installed = true;

    function emit(payload) {
        try {
            const w = window.__TAURI__;
            if (w && w.event && w.event.emit) {
                w.event.emit('forage-capture', payload);
                return;
            }
        } catch (e) { /* fall through */ }
        // Fallback for non-Tauri hosts: push to a global buffer the host
        // can poll via `JSON.stringify(window.__forage_captures)`.
        if (!window.__forage_captures) window.__forage_captures = [];
        window.__forage_captures.push(payload);
    }

    // --- fetch ---
    const origFetch = window.fetch;
    window.fetch = async function(...args) {
        const resp = await origFetch.apply(this, args);
        try {
            const cloned = resp.clone();
            const body = await cloned.text();
            const reqUrl = (typeof args[0] === 'string') ? args[0] : (args[0] && args[0].url) || resp.url;
            const method = (args[1] && args[1].method) || 'GET';
            emit({
                subkind: 'match',
                url: resp.url || reqUrl,
                method: method.toUpperCase(),
                status: resp.status,
                body,
            });
        } catch (e) {
            // Don't break the page.
        }
        return resp;
    };

    // --- XHR ---
    const origOpen = XMLHttpRequest.prototype.open;
    const origSend = XMLHttpRequest.prototype.send;
    XMLHttpRequest.prototype.open = function(method, url) {
        this.__forage_method = method;
        this.__forage_url = url;
        return origOpen.apply(this, arguments);
    };
    XMLHttpRequest.prototype.send = function() {
        this.addEventListener('load', () => {
            try {
                emit({
                    subkind: 'match',
                    url: this.responseURL || this.__forage_url || '',
                    method: (this.__forage_method || 'GET').toUpperCase(),
                    status: this.status,
                    body: typeof this.responseText === 'string' ? this.responseText : '',
                });
            } catch (e) { /* ignore */ }
        });
        return origSend.apply(this, arguments);
    };
})();
"#;

/// JS that scrolls the page to bottom; used by the host between settle
/// windows to drive `browserPaginate.scroll` recipes.
pub const SCROLL_TO_BOTTOM_JS: &str = r#"
window.scrollTo({ top: document.body.scrollHeight, behavior: 'instant' });
"#;

/// JS that reads `document.documentElement.outerHTML`. The host calls
/// this after settle to satisfy `captures.document`.
pub const DUMP_DOCUMENT_JS: &str = r#"
document.documentElement.outerHTML
"#;

/// JS overlay that an M10 interactive bootstrap injects: a fixed green
/// "✓ Scrape this page" button that emits a `forage-interactive-done`
/// event when clicked, signaling the human has solved the challenge.
pub const INTERACTIVE_OVERLAY_JS: &str = r#"
(() => {
    if (window.__forage_overlay) return;
    window.__forage_overlay = true;
    const btn = document.createElement('button');
    btn.id = '__forage_overlay';
    btn.textContent = '✓ Scrape this page';
    Object.assign(btn.style, {
        position: 'fixed',
        bottom: '24px',
        right: '24px',
        zIndex: 2147483647,
        padding: '12px 16px',
        background: '#10b981',
        color: 'white',
        border: 'none',
        borderRadius: '8px',
        fontFamily: 'system-ui, sans-serif',
        fontSize: '14px',
        cursor: 'pointer',
        boxShadow: '0 6px 16px rgba(0,0,0,0.25)',
    });
    btn.addEventListener('click', () => {
        try {
            const w = window.__TAURI__;
            if (w && w.event && w.event.emit) {
                w.event.emit('forage-interactive-done', {
                    url: window.location.href,
                });
            }
        } catch (e) {}
    });
    document.body && document.body.appendChild(btn);
})();
"#;
