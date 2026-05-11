import Foundation

/// JavaScript source strings that Forage's browser engine injects into a
/// `WKWebView` to (a) capture every fetch / XHR the page fires, (b) clear
/// common dismissable affordances (welcome modals, age gates), (c) drive
/// pagination by scrolling and clicking load-more buttons, and (d) dump the
/// page's interactable affordances for the diagnostic report.
///
/// All scripts are designed to run in the page's main world (the default for
/// `WKWebView.evaluateJavaScript`). They recurse into open shadow roots where
/// the SPA mounts (Jane uses `#shadow-host`); closed shadow roots are out of
/// reach by design.
public enum InjectedScripts {

    /// Document-start script that wraps `window.fetch` and `XMLHttpRequest` so
    /// every request + response is forwarded to the Swift host via a
    /// `WKScriptMessageHandler` named `captureNetwork`. Body capture is capped
    /// at 1 MB to bound memory.
    ///
    /// Forwarded payload shape (see `Capture.init(jsBridgePayload:)`):
    /// ```
    /// { kind: "fetch" | "xhr", method, requestUrl, responseUrl,
    ///   requestBody, status, body }
    /// ```
    public static let captureWrapper: String = #"""
    (function() {
        const send = function(payload) {
            try { window.webkit.messageHandlers.captureNetwork.postMessage(payload); } catch (e) {}
        };
        const stringifyBody = function(body) {
            if (body == null) return '';
            if (typeof body === 'string') return body;
            try {
                if (body instanceof FormData) {
                    const obj = {};
                    body.forEach((v, k) => { obj[k] = (typeof v === 'string') ? v : '<blob>'; });
                    return '__formdata__' + JSON.stringify(obj);
                }
                if (body instanceof URLSearchParams) return body.toString();
                if (body instanceof Blob) return '<Blob ' + body.size + 'B>';
                if (body instanceof ArrayBuffer) return '<ArrayBuffer ' + body.byteLength + 'B>';
                if (typeof body === 'object') return JSON.stringify(body);
                return String(body);
            } catch (e) { return '<unstringifiable>'; }
        };

        const origFetch = window.fetch;
        if (origFetch) {
            window.fetch = async function(...args) {
                const resp = await origFetch.apply(this, args);
                try {
                    const cloned = resp.clone();
                    const body = await cloned.text();
                    const reqUrl = (typeof args[0] === 'string') ? args[0] : (args[0] && args[0].url) || '';
                    const reqMethod = (args[1] && args[1].method)
                        || (typeof args[0] === 'object' && args[0] && args[0].method) || 'GET';
                    let reqBody = '';
                    if (args[1] && args[1].body !== undefined) reqBody = stringifyBody(args[1].body);
                    else if (typeof args[0] === 'object' && args[0] && args[0].body !== undefined) reqBody = stringifyBody(args[0].body);
                    send({
                        kind: 'fetch',
                        requestUrl: reqUrl,
                        responseUrl: resp.url,
                        method: reqMethod,
                        requestBody: (reqBody || '').slice(0, 200000),
                        status: resp.status,
                        body: (body || '').slice(0, 1000000)
                    });
                } catch (e) {}
                return resp;
            };
        }
        const OrigXHR = window.XMLHttpRequest;
        if (OrigXHR) {
            const origOpen = OrigXHR.prototype.open;
            const origSend = OrigXHR.prototype.send;
            OrigXHR.prototype.open = function(method, url) {
                this._captureMethod = method;
                this._captureUrl = url;
                return origOpen.apply(this, arguments);
            };
            OrigXHR.prototype.send = function(body) {
                const xhr = this;
                xhr._captureRequestBody = stringifyBody(body);
                xhr.addEventListener('loadend', function() {
                    try {
                        let body = '';
                        try { body = xhr.responseText || ''; } catch (e) {}
                        send({
                            kind: 'xhr',
                            requestUrl: xhr._captureUrl || '',
                            responseUrl: xhr.responseURL || xhr._captureUrl || '',
                            method: xhr._captureMethod || 'GET',
                            requestBody: (xhr._captureRequestBody || '').slice(0, 200000),
                            status: xhr.status,
                            body: body.slice(0, 1000000)
                        });
                    } catch (e) {}
                });
                return origSend.apply(this, arguments);
            };
        }
    })();
    """#

    /// Detects a DOB-style age-gate form (three inputs labelled month/day/year
    /// + a submit button), fills it with `1990-01-01`, and submits. Returns a
    /// non-empty string if the form was submitted; empty string otherwise.
    /// Recipes typically follow up with a `webView.reload()` so the SPA boots
    /// fresh on the post-gate page (some plugins intercept submit via AJAX
    /// rather than letting the form do a real POST navigation).
    public static let ageGateFill: String = #"""
    (function() {
        const findForm = () => {
            for (const f of document.querySelectorAll('form')) {
                const sig = ((f.className||'') + ' ' + (f.id||'') + ' ' + (f.action||'')).toLowerCase();
                if (/age[\-_ ]?gate|age[\-_ ]?verif|agegate/.test(sig)) return f;
            }
            for (const f of document.querySelectorAll('form')) {
                let m=false, d=false, y=false;
                for (const i of f.querySelectorAll('input')) {
                    const t = ((i.placeholder||'')+' '+(i.name||'')+' '+(i.id||'')+' '+(i.getAttribute('aria-label')||'')).toLowerCase();
                    if (!m && (/\bmm\b/.test(t) || /\bmonth\b/.test(t) || /\[m\]/.test(t))) m = true;
                    else if (!d && (/\bdd\b/.test(t) || /\bday\b/.test(t) || /\[d\]/.test(t))) d = true;
                    else if (!y && (/\byyyy?\b/.test(t) || /\byear\b/.test(t) || /\[y\]/.test(t))) y = true;
                }
                if (m && d && y) return f;
            }
            return null;
        };
        const setVal = (el, v) => {
            el.focus();
            el.value = v;
            el.dispatchEvent(new Event('input', { bubbles: true }));
            el.dispatchEvent(new Event('change', { bubbles: true }));
        };
        const form = findForm();
        if (!form) return '';
        let mEl=null, dEl=null, yEl=null;
        for (const i of form.querySelectorAll('input')) {
            if (i.type === 'hidden') continue;
            const t = ((i.placeholder||'')+' '+(i.name||'')+' '+(i.id||'')+' '+(i.getAttribute('aria-label')||'')).toLowerCase();
            if (!mEl && (/\bmm\b/.test(t) || /\bmonth\b/.test(t) || /\[m\]/.test(t))) mEl = i;
            else if (!dEl && (/\bdd\b/.test(t) || /\bday\b/.test(t) || /\[d\]/.test(t))) dEl = i;
            else if (!yEl && (/\byyyy?\b/.test(t) || /\byear\b/.test(t) || /\[y\]/.test(t))) yEl = i;
        }
        if (!(mEl && dEl && yEl)) return '';
        setVal(mEl, '01');
        setVal(dEl, '01');
        setVal(yEl, '1990');
        const submitBtn = form.querySelector('button[type=submit], input[type=submit]');
        if (submitBtn) submitBtn.click();
        else form.submit();
        return 'submitted age-gate (1990-01-01)';
    })();
    """#

    /// Click-to-dismiss heuristic: walks document + open shadow roots,
    /// prioritizes age-gate confirmations (substring match for verbose
    /// phrasings like "Yes, I am 21 or older") then exact-match welcome
    /// dismissals (`Close`, `I agree`, etc). Returns the clicked element's
    /// text or empty string if nothing matched.
    public static let dismissModal: String = #"""
    (function() {
        const ageGateSubstrings = [
            'i am 21 or older', 'yes, i am 21', 'i am 21+', 'i am 21 and over',
            'i am over 21', 'i am 18 or older', 'yes, i am 18', 'i am 18+',
            'i am 18 and over', 'i am over 18', 'i am of legal age',
            'enter the site', 'continue to site', 'i confirm i am'
        ];
        const exactDismissals = [
            'I agree', 'Accept', 'Accept all', 'Got it', 'OK', 'Confirm',
            'Continue', 'Enter', 'Enter site', 'Yes',
            'No thanks', 'Skip', 'Dismiss', 'Close'
        ];

        const findIn = (root) => {
            const candidates = root.querySelectorAll('button, a[role=button], [role=button], a, div[onclick]');
            for (const el of candidates) {
                const text = (el.textContent || '').trim().toLowerCase();
                if (!text || text.length > 200) continue;
                for (const s of ageGateSubstrings) {
                    if (text.includes(s)) {
                        el.click();
                        return el.textContent.trim().slice(0, 80);
                    }
                }
            }
            for (const el of candidates) {
                const text = (el.textContent || '').trim();
                if (!text) continue;
                for (const t of exactDismissals) {
                    if (text === t || text.toLowerCase() === t.toLowerCase()) {
                        el.click();
                        return text;
                    }
                }
            }
            const all = root.querySelectorAll('*');
            for (const e of all) {
                if (e.shadowRoot) {
                    const found = findIn(e.shadowRoot);
                    if (found) return found;
                }
            }
            return null;
        };
        return findIn(document) || '';
    })();
    """#

    /// "Drive the SPA forward" — the workhorse for `BrowserPaginate { mode: scroll }`.
    /// Each iteration:
    /// 1. Scrolls the window + every nested shadow-DOM scrollable to the bottom
    ///    (lazy-load via IntersectionObserver / scroll events).
    /// 2. Clicks the bottom-most visible button matching common load-more
    ///    labels (`Shop all products`, `View more`, `Show more`, `Load more`,
    ///    `View all`, `See more`). Position-disambiguates when a banner shares
    ///    a label with the pagination button (after `scrollTo(bottom)`, the
    ///    pagination button is the one with the highest viewport `top`).
    public static let scrollAndClickLoadMore: String = #"""
    (function() {
        try { window.scrollTo({ top: document.body.scrollHeight, behavior: 'auto' }); }
        catch (e) { try { window.scrollTo(0, document.body.scrollHeight); } catch (_) {} }
        const collect = (root, acc) => {
            try {
                for (const el of root.querySelectorAll('*')) {
                    try {
                        if (el.scrollHeight > el.clientHeight + 8) acc.push(el);
                        if (el.shadowRoot) collect(el.shadowRoot, acc);
                    } catch (_) {}
                }
            } catch (_) {}
        };
        const acc = [];
        collect(document, acc);
        for (const s of acc) {
            try { s.scrollTop = s.scrollHeight; } catch (e) {}
        }
        try { window.dispatchEvent(new Event('scroll')); } catch (e) {}

        const loadMoreLabels = ['shop all products', 'show more', 'view more', 'load more', 'see more', 'view all'];
        const matches = [];
        const findInRoot = (root) => {
            let nodes;
            try { nodes = root.querySelectorAll('button, a, [role=button]'); } catch (_) { return; }
            for (const el of nodes) {
                const text = (el.textContent || '').trim().toLowerCase();
                if (!loadMoreLabels.includes(text)) continue;
                const r = el.getBoundingClientRect && el.getBoundingClientRect();
                if (!r || r.width === 0 || r.height === 0) continue;
                matches.push({ el, top: r.top, label: (el.textContent || '').trim() });
            }
            try {
                for (const e of root.querySelectorAll('*')) {
                    if (e.shadowRoot) findInRoot(e.shadowRoot);
                }
            } catch (_) {}
        };
        findInRoot(document);
        matches.sort((a, b) => b.top - a.top);
        let clicked = "";
        if (matches.length > 0) {
            try {
                matches[0].el.click();
                clicked = " clicked '" + matches[0].label + "' (top=" + Math.round(matches[0].top) + ", " + matches.length + " candidates)";
            } catch (e) {}
        }
        return "scrolled " + acc.length + " containers" + clicked;
    })();
    """#

    /// Walks document + open shadow roots, returns a JSON-encoded summary of
    /// every visible button / link / role=button / scrollable container with
    /// its label and a CSS-style path. Mirrors the eventual diagnostic
    /// report's "unhandled UI affordances" section — what *could* a recipe
    /// have driven that it didn't?
    public static let dumpAffordances: String = #"""
    (function() {
        const out = { buttons: [], links: [], roleButtons: [], scrollables: [], inputs: [] };
        const seen = new Set();
        const walk = (root) => {
            let nodes;
            try { nodes = root.querySelectorAll('button, a, [role=button], input, select, [data-href], [onclick]'); }
            catch (e) { return; }
            for (const el of nodes) {
                if (seen.has(el)) continue; seen.add(el);
                const rect = (el.getBoundingClientRect && el.getBoundingClientRect()) || {width:0, height:0, top:0, left:0};
                const visible = rect.width > 0 && rect.height > 0;
                if (!visible) continue;
                const text = (el.textContent || '').trim().replace(/\s+/g, ' ').slice(0, 120);
                const tag = el.tagName.toLowerCase();
                const cls = el.className && typeof el.className === 'string' ? el.className.split(/\s+/).slice(0, 4).join('.') : '';
                const id = el.id || '';
                const sel = tag + (id ? `#${id}` : '') + (cls ? `.${cls}` : '');
                const item = { selector: sel, text, x: Math.round(rect.left), y: Math.round(rect.top) };
                if (tag === 'a') {
                    const href = el.getAttribute('href') || '';
                    if (href) item.href = href;
                    out.links.push(item);
                } else if (tag === 'button') {
                    out.buttons.push(item);
                } else if (el.getAttribute && el.getAttribute('role') === 'button') {
                    out.roleButtons.push(item);
                } else if (tag === 'input' || tag === 'select') {
                    item.type = el.getAttribute('type') || tag;
                    item.name = el.getAttribute('name') || '';
                    out.inputs.push(item);
                }
            }
            let all;
            try { all = root.querySelectorAll('*'); } catch (e) { return; }
            for (const el of all) {
                try {
                    if (el.scrollHeight > el.clientHeight + 8 && el.clientHeight > 50) {
                        const cls = el.className && typeof el.className === 'string' ? el.className.split(/\s+/).slice(0, 3).join('.') : '';
                        const tag = el.tagName.toLowerCase();
                        const sel = tag + (el.id ? `#${el.id}` : '') + (cls ? `.${cls}` : '');
                        out.scrollables.push({ selector: sel, scrollHeight: el.scrollHeight, clientHeight: el.clientHeight });
                    }
                    if (el.shadowRoot) walk(el.shadowRoot);
                } catch (_) {}
            }
        };
        walk(document);
        return JSON.stringify(out);
    })();
    """#

    /// Build replay JS: parse the seed body as JSON, apply dotted-path
    /// overrides (with `$i` substitution for the iteration number), JSON-encode,
    /// fire fetch via the page context. Used by `BrowserPaginate { mode: replay }`.
    public static func replayFork(url: String, seedBody: String, override: [String: Any], iter: Int) -> String {
        let urlLit = jsString(url)
        let seedLit = jsString(seedBody)
        var overrideEntries: [String] = []
        for (path, value) in override {
            let resolved = resolveTemplate(value, iter: iter)
            let valLit: String
            if let s = resolved as? String {
                valLit = jsString(s)
            } else if JSONSerialization.isValidJSONObject([resolved]),
                      let data = try? JSONSerialization.data(withJSONObject: [resolved]),
                      let txt = String(data: data, encoding: .utf8) {
                valLit = String(txt.dropFirst().dropLast())
            } else {
                valLit = "null"
            }
            overrideEntries.append("[\(jsString(path)), \(valLit)]")
        }
        let overrideJS = "[\(overrideEntries.joined(separator: ", "))]"
        return #"""
        (function() {
            const reportDiag = (msg) => {
                try {
                    window.webkit.messageHandlers.captureNetwork.postMessage({
                        kind: "diagnostic",
                        requestUrl: "paginate-replay",
                        responseUrl: "paginate-replay",
                        method: "DIAG",
                        requestBody: "",
                        status: -1,
                        body: "[paginate-replay iter=\#(iter)] " + msg
                    });
                } catch (e) {}
            };
            const url = \#(urlLit);
            let body;
            try { body = JSON.parse(\#(seedLit)); } catch (e) { reportDiag("seed not JSON"); return "seed-not-json"; }
            const overrides = \#(overrideJS);
            const setPath = (obj, path, val) => {
                const parts = path.split('.');
                let cur = obj;
                for (let i = 0; i < parts.length - 1; i++) {
                    const k = parts[i];
                    const idx = parseInt(k, 10);
                    if (!isNaN(idx) && Array.isArray(cur)) cur = cur[idx];
                    else { if (cur[k] == null) cur[k] = {}; cur = cur[k]; }
                }
                const last = parts[parts.length - 1];
                const lastIdx = parseInt(last, 10);
                if (!isNaN(lastIdx) && Array.isArray(cur)) cur[lastIdx] = val;
                else cur[last] = val;
            };
            for (const [p, v] of overrides) setPath(body, p, v);
            (async () => {
                try {
                    const resp = await fetch(url, {
                        method: "POST",
                        headers: {"Content-Type": "application/json"},
                        body: JSON.stringify(body),
                        credentials: "include"
                    });
                    reportDiag("status=" + resp.status + " ok=" + resp.ok);
                } catch (e) {
                    reportDiag("threw: " + e + " (name=" + (e && e.name) + ")");
                }
            })();
            return "fired replay iter=" + \#(iter);
        })();
        """#
    }

    /// Click a single button by exact (case-sensitive) text content. Used by
    /// pre-pagination "warmup" navigation steps in recipes (e.g. clicking
    /// `All products` to navigate the SPA into the paginatable view).
    public static func clickButtonByText(_ text: String) -> String {
        let escaped = text.replacingOccurrences(of: "\"", with: "\\\"")
        return #"""
        (function(text) {
            const findIn = (root) => {
                let nodes;
                try { nodes = root.querySelectorAll('button, a, [role=button]'); } catch(_) { return null; }
                for (const el of nodes) {
                    if ((el.textContent || '').trim() === text) {
                        const r = el.getBoundingClientRect && el.getBoundingClientRect();
                        if (r && r.width > 0 && r.height > 0) return el;
                    }
                }
                try {
                    for (const e of root.querySelectorAll('*')) {
                        if (e.shadowRoot) {
                            const found = findIn(e.shadowRoot);
                            if (found) return found;
                        }
                    }
                } catch(_) {}
                return null;
            };
            const btn = findIn(document);
            if (btn) { btn.click(); return "clicked"; }
            return "not-found";
        })("\#(escaped)")
        """#
    }

    /// Floating "Scrape this page" overlay button for M10 interactive
    /// bootstrap. Injected after navigation finishes when the recipe
    /// declares `browser.interactive { … }`. Posts a message via the
    /// `forageInteractiveDone` `WKScriptMessageHandler` when clicked,
    /// signaling that the human has cleared whatever gate the page put
    /// up and the engine should snapshot the document + persist cookies.
    ///
    /// The overlay is the *only* affordance the user needs to find on a
    /// site they've never seen before — bright accent color, fixed
    /// position, max z-index, idempotent injection (re-running the
    /// script is a no-op).
    public static let interactiveOverlay: String = #"""
    (function() {
        if (document.getElementById('__forage-scrape-btn__')) return;
        const btn = document.createElement('button');
        btn.id = '__forage-scrape-btn__';
        btn.textContent = '✓ Scrape this page';
        btn.style.cssText = [
            'position:fixed',
            'bottom:24px',
            'right:24px',
            'z-index:2147483647',
            'padding:14px 22px',
            'background:#5c8a4f',
            'color:#fff',
            'border:none',
            'border-radius:10px',
            'font-family:-apple-system,BlinkMacSystemFont,sans-serif',
            'font-size:14px',
            'font-weight:600',
            'cursor:pointer',
            'box-shadow:0 4px 16px rgba(0,0,0,0.35)',
        ].join(';');
        btn.addEventListener('mouseenter', () => { btn.style.background = '#6a9b5c'; });
        btn.addEventListener('mouseleave', () => { btn.style.background = '#5c8a4f'; });
        btn.addEventListener('click', () => {
            btn.disabled = true;
            btn.textContent = 'Capturing…';
            btn.style.background = '#4d7843';
            try {
                window.webkit.messageHandlers.forageInteractiveDone.postMessage({
                    url: location.href,
                    html: document.documentElement.outerHTML,
                });
            } catch (e) {
                btn.textContent = 'Error: ' + (e && e.message ? e.message : 'no handler');
                btn.disabled = false;
            }
        });
        document.body.appendChild(btn);
    })();
    """#

    /// Dump `window.localStorage` as a JSON object. Used at interactive
    /// bootstrap completion to capture per-origin state. Returns an
    /// empty object when access is denied (cross-origin frame, file://,
    /// etc.).
    public static let dumpLocalStorage: String = #"""
    (function() {
        try {
            const out = {};
            for (let i = 0; i < localStorage.length; i++) {
                const k = localStorage.key(i);
                if (k != null) out[k] = localStorage.getItem(k) || '';
            }
            return JSON.stringify(out);
        } catch (e) {
            return '{}';
        }
    })();
    """#

    /// Restore a `localStorage` snapshot for the current origin.
    /// Invoked after navigation on headless re-runs of an interactive
    /// recipe. The snapshot is a `[String: String]` JSON object.
    public static func restoreLocalStorage(_ json: String) -> String {
        let escaped = jsString(json)
        return #"""
        (function(jsonText) {
            try {
                const data = JSON.parse(jsonText);
                for (const k of Object.keys(data)) {
                    localStorage.setItem(k, data[k]);
                }
                return 'ok';
            } catch (e) {
                return 'err:' + (e && e.message ? e.message : 'unknown');
            }
        })("\#(escaped)")
        """#
    }

    // MARK: - Helpers

    private static func resolveTemplate(_ value: Any, iter: Int) -> Any {
        if let s = value as? String {
            if s == "$i" { return iter }
            if s.contains("$i") { return s.replacingOccurrences(of: "$i", with: String(iter)) }
        }
        return value
    }

    private static func jsString(_ s: String) -> String {
        // Encode as a JSON string literal (handles all escaping).
        if let data = try? JSONSerialization.data(withJSONObject: [s], options: []),
           let text = String(data: data, encoding: .utf8) {
            return String(text.dropFirst().dropLast())
        }
        return "\"\""
    }
}
