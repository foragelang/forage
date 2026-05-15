# Interactive bootstrap (M10)

Some sites escalate to a human-verification challenge (CAPTCHA, "I'm
not a robot," interactive puzzle) before showing the menu. Forage
doesn't try to defeat these programmatically. M10's
`browser.interactive { … }` block hands off to a person once; the
resulting session is reused headlessly until it expires.

## Recipe-side

```forage
browser {
    initialURL: "https://www.ebay.com/sch/i.html?_nkw={$input.query}&LH_Sold=1"
    observe:    "ebay.com/sch"
    paginate browserPaginate.scroll {
        until: noProgressFor(2)
    }

    interactive {
        cookieDomains:         ["ebay.com", ".ebay.com"]
        sessionExpiredPattern: "Security Measure"
    }

    captures.document {
        for $card in $ | select("li.s-item") {
            emit SoldListing {
                title     ← $card | select(".s-item__title span") | text
                soldPrice ← $card | select(".s-item__price") | text
                url       ← $card | select("a.s-item__link") | attr("href")
            }
        }
    }
}
```

- `cookieDomains` — which cookies are persisted. Empty = every cookie
  from the bootstrap URL's host.
- `sessionExpiredPattern` — the literal text the target shows when our
  session is no longer valid (eBay shows `"Security Measure"` when the
  Akamai challenge re-prompts). When the engine sees this in the
  rendered HTML on a reuse run, it evicts the cached session and asks
  the user to re-run with `--interactive`. It's a **re-prompt signal,
  not a bypass hook** — Forage doesn't try to solve the verification
  itself.

## First run

```sh
forage run --interactive ~/Library/Forage/Recipes/ebay-sold --input query=polaroid+sx-70
```

Studio prefers the menu item **Recipe → Bootstrap session…**.

Either path:

1. Opens a visible WebView at the recipe's `bootstrapURL` (defaults to
   `initialURL`).
2. The human solves the challenge in the normal browser flow.
3. The engine injects a "✓ Scrape this page" overlay; clicking it tells
   the engine "you can take it from here."
4. Cookies (filtered by `cookieDomains`) + per-origin localStorage
   snapshot into `~/Library/Forage/Sessions/<slug>/session.json`
   (chmod 600).
5. The engine immediately proceeds with the recipe headlessly using
   that session.

## Subsequent runs

```sh
forage run ~/Library/Forage/Recipes/ebay-sold --input query=polaroid+sx-70
```

The engine seeds the cookies + localStorage back into the WebView,
navigates to `initialURL`, scans the rendered HTML for
`sessionExpiredPattern`:

- **Match found** → session expired. Engine evicts the cache and emits
  `stallReason: "session-expired: re-run with --interactive"`.
- **No match** → engine proceeds with normal pagination + captures.

## Boundaries

- We pass JS-execution checks (Cloudflare's basic challenge, Akamai's
  basic fingerprint check) **because we're a real browser engine**
  (WKWebView / WebView2 / WebKitGTK), not because we spoof anything.
  Honest UA, no IP rotation, no googlebot impersonation.
- We don't solve CAPTCHA / human-verification programmatically — M10
  exists precisely so we don't have to.
- Login walls / account-required pages are out of scope. If a site
  requires sign-in, the human signs in during the interactive
  bootstrap; the session is reused.

The interactive bootstrap was previously discussed and accepted as the
right posture for Jane (Trilogy) and now eBay; new platforms inherit
the same pattern.
