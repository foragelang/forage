# Authenticated sessions

Forage's `auth.session.*` block adds first-class support for "log in as me, maintain a session across requests, refresh when it expires." The runtime drives the login flow before the recipe body runs, threads cookies or bearer tokens into every subsequent step, and re-authenticates on a single `401`/`403` before giving up.

Three variants, picked per recipe:

- `auth.session.formLogin` — POST credentials to a login endpoint; capture `Set-Cookie`; reuse on subsequent requests.
- `auth.session.bearerLogin` — POST credentials to a token endpoint; extract a token from the response; inject `Authorization: Bearer <token>` on subsequent requests.
- `auth.session.cookiePersist` — load cookies from a file. Escape hatch for sites whose login the recipe can't drive (multi-device MFA, embedded CAPTCHAs).

## Where credentials live

**Never in the recipe text.** Recipes reference credentials via `$secret.<name>`. The runtime resolves these at execution time:

| Host        | Source                                                                   |
| ----------- | ------------------------------------------------------------------------ |
| CLI         | `FORAGE_SECRET_<NAME>` environment variables                             |
| Toolkit     | Same env vars by default; future versions can swap in a Keychain-backed resolver |
| Web IDE     | Not supported — sessioned recipes refuse to run in-browser               |

Declare each secret the recipe needs at the top:

```forage
recipe "example" {
    engine http
    secret username
    secret password
    ...
}
```

The validator emits a warning if a recipe references `$secret.foo` without declaring `secret foo`, and another if a declared secret is never used. Both catch typos cleanly.

## formLogin

```forage
recipe "site-with-cookies" {
    engine http
    secret username
    secret password
    type Item { id: String }

    auth.session.formLogin {
        url:    "https://example.com/login"
        method: "POST"
        body.form {
            "username": $secret.username
            "password": $secret.password
        }
        captureCookies: true
        maxReauthRetries: 1
        cache: 3600         // seconds; omit to disable caching
    }

    step items { method "GET"; url "https://example.com/items" }
    for $it in $items {
        emit Item { id ← $it.id }
    }
}
```

The runtime:

1. POSTs the rendered body to `url` with no session injected.
2. Parses `Set-Cookie` headers from the response into a cookie jar.
3. Adds `Cookie: name=value; name2=value2` to every subsequent step request.

If `items` returns `401`/`403`, the engine drops the cached session, re-runs the login, and retries the original request once. A second `401`/`403` becomes `stallReason: "auth-failed: HTTP 401 after re-auth"` and the run stops.

## bearerLogin

```forage
recipe "oauth-style" {
    engine http
    secret clientId
    secret clientSecret
    type Item { id: String }

    auth.session.bearerLogin {
        url: "https://example.com/oauth/token"
        body.json {
            client_id:     $secret.clientId
            client_secret: $secret.clientSecret
            grant_type:    "client_credentials"
        }
        tokenPath:    $.access_token
        headerName:   "Authorization"     // default
        headerPrefix: "Bearer "           // default
    }

    step items { method "GET"; url "https://example.com/items" }
    for $it in $items {
        emit Item { id ← $it.id }
    }
}
```

The runtime extracts `$.access_token` from the login response JSON and adds `Authorization: Bearer <token>` to every subsequent step request. Override `headerName` / `headerPrefix` for sites that expect a different shape (e.g. `X-Api-Token: <token>` with `headerPrefix: ""`).

## cookiePersist

For sites whose login flow the recipe can't drive — interactive MFA across multiple devices, an embedded CAPTCHA, anything where a human has to click — manage the session externally and point Forage at the resulting cookie file:

```forage
recipe "escape-hatch" {
    engine http
    secret cookieFile
    type Item { id: String }

    auth.session.cookiePersist {
        sourcePath: "{$secret.cookieFile}"
        format:     json
    }

    step items { method "GET"; url "https://example.com/items" }
    for $it in $items {
        emit Item { id ← $it.id }
    }
}
```

`format` accepts:

- `json` — `[{"name": "...", "value": "...", "domain": "...", "path": "..."}, ...]`
- `netscape` — the `cookies.txt` format browser exporters typically produce.

`cookiePersist` doesn't re-authenticate on 401: it has no credentials to retry with. Refresh the file out-of-band and re-run.

## MFA

Recipes whose login requires a second-factor code declare it:

```forage
auth.session.formLogin {
    url:    "https://example.com/login"
    body.form {
        "username": $secret.username
        "password": $secret.password
    }
    requiresMFA: true
    mfaFieldName: "otp"     // default "code"
}
```

When the engine reaches the login step it pauses, asks the host for a code, and re-sends the login with `<mfaFieldName>: <code>` appended to the body.

| Host        | Prompt                                                    |
| ----------- | --------------------------------------------------------- |
| CLI         | Stderr prompt; one line from stdin. Pass `--no-mfa` to disable. |
| Toolkit     | Modal sheet with a SecureField.                           |
| Web IDE     | Not supported.                                            |

If the user cancels, `stallReason` becomes `auth-mfa-cancelled` and the run stops.

## Caching

`cache: <seconds>` persists the resolved session (cookies or bearer token) to `~/Library/Forage/Cache/sessions/<recipe-name>/<credential-fingerprint>.json`. Subsequent runs within the window skip the login and inherit the session.

Cache file properties:

- **`chmod 600`** — readable only by the owning user.
- Filename includes a SHA-256 fingerprint over the resolved credential values, so a rotation produces a fresh cache entry instead of mixing with stale state.
- A mid-run `401`/`403` evicts the cache and re-runs the login.
- An expired cache is skipped; the runtime re-logs in and writes a fresh entry.

Optional `cacheEncrypted: true` opts into AES-GCM encryption of the file at rest, keyed by a per-machine secret held by the host. The current v1 host doesn't ship a key supplier, so encryption is a no-op — the file is still `chmod 600`. Future hosts will wire in a Keychain-backed key.

## Security notes

- **Credentials are never logged.** The engine maintains a `SecretRedactor` over the resolved values and scrubs every diagnostic string it emits. If an HTTP error message accidentally echoes a credential value back, the value is replaced with `<redacted>` before it lands in `stallReason`. (Values shorter than 4 characters are not redacted — single-character substitution would corrupt unrelated output.)
- **Cache files are never world-readable.** The runtime enforces `chmod 600` at write time.
- **The web IDE refuses to run sessioned recipes.** Even when an in-browser fetch could succeed, persisting credentials to localStorage isn't viable. Use the CLI or Toolkit.

## Diagnostic envelopes

The `auth.session.*` runtime introduces three new `DiagnosticReport.stallReason` prefixes:

| Stall reason                       | Meaning                                                                            |
| ---------------------------------- | ---------------------------------------------------------------------------------- |
| `auth-failed: <detail>`            | Login or re-auth failed (4xx response, empty cookies, missing `tokenPath`, …)      |
| `auth-mfa-cancelled`               | The user cancelled the MFA prompt.                                                 |
| `auth-secret-missing: <name>`      | The recipe referenced `$secret.<name>` but the resolver had no value to give back. |
