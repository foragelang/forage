# Auth

HTTP-engine auth runs before any data step. Five flavors:

## `auth.staticHeader`

A constant header on every request. Templates resolve at runtime; the
header rebuilds per request only if the template references a value
that's changed (it usually doesn't).

```forage
auth.staticHeader {
    name:  "storeId"
    value: "{$input.storeId}"
}
```

## `auth.htmlPrime`

Fetch an HTML page once before the rest of the body. Cookies set on
that fetch live in the engine's cookie jar for the rest of the run;
regex captures from the response body populate scope variables the
later steps reference.

```forage
auth.htmlPrime {
    step:       prime
    nonceVar:   "ajaxNonce"
    ajaxUrlVar: "ajaxUrl"
}

step prime {
    method "GET"
    url    "{$input.menuPageURL}"
    extract.regex {
        pattern: "leafbridge_public_ajax_obj\\s*=\\s*\\{\"ajaxurl\":\"([^\"]+)\",\"nonce\":\"([a-f0-9]+)\"\\}"
        groups:  [ajaxUrl, ajaxNonce]
    }
}
```

After `prime` runs, `{$ajaxUrl}` and `{$ajaxNonce}` are template-usable
in every subsequent step.

## `auth.session.formLogin`

POST credentials to a login endpoint; cookies in the response thread
forward via the engine's cookie jar.

```forage
auth.session.formLogin {
    url:               "https://example.com/api/login"
    method:            "POST"
    body.form {
        "username": $secret.username
        "password": $secret.password
    }
    captureCookies:    true
    maxReauthRetries:  1
    cache:             600        // seconds; cache the resolved session
}
```

`cache` opts into the on-disk session cache (chmod 600,
`~/Library/Forage/Cache/sessions/<recipe>/<fingerprint>.json`). The
fingerprint is sha-256 over the resolved credential values; rotating
either secret produces a fresh cache entry. On a mid-run `401`, the
engine evicts the cache and re-runs the login.

## `auth.session.bearerLogin`

POST credentials, extract a token from the response body, inject it as
a `Bearer` header on every subsequent step.

```forage
auth.session.bearerLogin {
    url:    "https://example.com/oauth/token"
    method: "POST"
    body.json {
        client_id:     $secret.clientId
        client_secret: $secret.clientSecret
        grant_type:    "client_credentials"
    }
    tokenPath:    $.access_token
    headerName:   "Authorization"
    headerPrefix: "Bearer "
}
```

## `auth.session.cookiePersist`

Escape hatch: load cookies from a file. Useful for sites where the
recipe can't drive the login (cross-device MFA, embedded CAPTCHA).

```forage
auth.session.cookiePersist {
    sourcePath: "{$input.sessionPath}"
    format:     json                  // or netscape
}
```

## MFA

Any `session.*` variant can opt into MFA:

```forage
auth.session.formLogin {
    // …body, url, method…
    requiresMFA:   true
    mfaFieldName:  "code"
}
```

The engine pauses, calls the host's `MFAProvider`, and re-sends the
login with `<mfaFieldName>: <code>` added to the body.

- CLI: stdin prompt.
- Studio: modal sheet with a SecureField.
- Web IDE: refuses sessioned recipes outright.

If the user cancels, the engine surfaces `stallReason:
"auth-mfa-cancelled"` and stops.
