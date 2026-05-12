# Hub authentication

The hub uses GitHub as its identity provider. Three OAuth flows cover
every client:

| Flow | Used by |
|---|---|
| Web (authorization code) | `hub.foragelang.com` web IDE |
| Device code | `forage auth login` CLI + Forage Studio Sign-in sheet |
| Refresh | every long-lived client refreshing an expired access token |

## Token shape

Access tokens are HS256 JWTs signed with the Worker's
`JWT_SIGNING_KEY` secret. Access tokens expire in 1 hour; refresh
tokens expire in 30 days. Different audiences (`forage-hub:access` vs
`forage-hub:refresh`) so an access verifier rejects a refresh token
and vice versa.

The hub doesn't trust client-presented refresh tokens directly — each
refresh token stored server-side carries a fingerprint; clients send
the JWT, the server matches the fingerprint, rotates if necessary.

## Device-code flow

```sh
forage auth login
```

Under the hood:

1. CLI POSTs `/v1/oauth/device` → `{user_code, verification_url,
   device_code, interval, expires_in}`.
2. CLI prints the user code + URL; user opens the URL on any device,
   types the code, signs in with GitHub.
3. CLI polls `/v1/oauth/device/poll` every `interval` seconds. While
   pending, the server returns `202 {status: "pending"}`. On success,
   `200 {status: "ok", access_token, refresh_token, user: {login}}`.
4. CLI writes the tokens to
   `~/Library/Forage/Auth/api.foragelang.com.json` (chmod 600).

Same flow drives Forage Studio's **Sign in with GitHub** sheet.

## Web flow

`hub.foragelang.com/edit` → **Sign in with GitHub** kicks off the
authorization-code flow:

1. Browser → `/v1/oauth/start?returnTo=<url>` redirects to GitHub.
2. GitHub redirects back to `/v1/oauth/callback?code=…` with the
   auth code.
3. Worker exchanges the code for an access token, looks up the user,
   mints a Forage JWT, sets it as an httpOnly cookie (`forage_at`),
   then redirects to the original `returnTo`.

Subsequent IDE requests carry the cookie; the Worker's
`identifyCaller` recognizes the cookie OR an `Authorization: Bearer`
header — clients pick whichever fits.

## Refresh

```http
POST /v1/oauth/refresh
{ "refreshToken": "<jwt>" }
```

Returns a new `{access_token, refresh_token, expires_in}` pair. The
old refresh token is invalidated (fingerprint rotated server-side) so
a stolen old token can't be used after the legitimate client refreshes.

## Revoke

```sh
forage auth logout --revoke
```

POSTs `/v1/oauth/revoke`, the Worker clears the stored refresh
fingerprint. The local auth-store file is deleted regardless.

## OAuth activation

The endpoints all ship in the Worker and return `503
oauth_not_configured` until the operator registers the GitHub OAuth
App and adds the secrets:

```sh
wrangler secret put GITHUB_OAUTH_CLIENT_ID
wrangler secret put GITHUB_OAUTH_CLIENT_SECRET
wrangler secret put JWT_SIGNING_KEY
```

(See [RELEASING](../../../RELEASING.md) for the full activation
checklist.)
