# Sessions and caching

Anything `auth.session.*` produces is a session: a cookie jar, a bearer
token, or a loaded cookie file. The HTTP engine resolves the session
once per run before any data step, threads it through every subsequent
request, and (optionally) persists it across runs.

## In-memory session

By default the session lives only inside one run:

- `formLogin` → cookies in `reqwest`'s cookie store, attached on
  follow-up requests by URL host.
- `bearerLogin` → `Authorization: Bearer <token>` injected by the
  engine's `apply_request_headers` step.
- `cookiePersist` → cookies read from disk at startup, attached to the
  jar.

The engine re-runs the login automatically on a mid-run 401/403, up to
`maxReauthRetries` times (default 1).

## Persisted cache

Opt into the on-disk session cache by setting `cache: <seconds>`:

```forage
auth.session.formLogin {
    url: "https://example.com/api/login"
    body.form { "username": $secret.username, "password": $secret.password }
    cache: 3600   // reuse the resolved session for an hour
}
```

The cache lands at
`~/Library/Forage/Cache/sessions/<recipe-slug>/<fingerprint>.json`
(`$XDG_CACHE_HOME` on Linux, `%LOCALAPPDATA%` on Windows). Properties:

- **`chmod 600`** — readable only by the owning user. Enforced at write
  time on Unix.
- **Fingerprint = SHA-256** over the resolved credential values. A
  rotated password produces a fresh cache entry rather than mixing
  with stale state.
- **Mid-run 401/403** evicts the cache and re-runs the login.
- **Expired cache** is skipped — the engine runs the login again and
  writes a fresh entry.

## Encryption at rest

`cacheEncrypted: true` opts into AES-GCM encryption, keyed by a
per-machine secret stored in the OS keychain (`forage-keychain`). The
file remains chmod 600 either way. Without a key supplier (CI runners
without a desktop session), the engine falls back to plain JSON +
chmod 600.

## Security model

- **Credentials never log.** `SecretRedactor` scrubs every diagnostic
  message of resolved credential values. Errors that echo back a
  credential are replaced with `<redacted>` before they leave the
  process. Values shorter than 4 characters aren't redacted — single
  characters would chew up unrelated output.
- **The CLI never writes credentials to disk.** Plaintext credentials
  live only in `$secret.<name>` resolution scope and the request
  bodies the engine builds.
- **The web IDE refuses sessioned recipes outright.** Persisting
  credentials to localStorage from a browser context isn't viable;
  use the CLI or Forage Studio.
