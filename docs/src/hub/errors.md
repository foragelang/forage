# Error envelope

Every non-2xx response from `api.foragelang.com` carries the same
shape:

```json
{
    "error": {
        "code":    "<machine_code>",
        "message": "<human prose>",
        // optional, code-specific:
        "retryAfter": 47,
        "details":    { … }
    }
}
```

The `code` is what clients should branch on; the `message` is for the
human reading the log. Optional fields layer on per code.

## Codes

| Status | Code | Meaning |
|---|---|---|
| 400 | `bad_json` | Request body wasn't valid JSON |
| 400 | `bad_slug` | Slug format violation (`<namespace>/<name>`) |
| 400 | `invalid` | Recipe failed structural validation |
| 401 | `unauthorized` | Missing or invalid bearer token |
| 403 | `forbidden` | Authenticated but not allowed (ownership) |
| 404 | `not_found` | Slug doesn't exist (or version doesn't) |
| 404 | `no_route` | No handler for the path + method |
| 405 | `method_not_allowed` | Verb not supported for this route |
| 413 | `payload_too_large` | Request envelope > 16 MiB |
| 413 | `recipe_too_large` | Recipe source > 1 MiB |
| 429 | `rate_limited` | Bucket exceeded; `retryAfter` carries seconds |
| 500 | `internal` | Unhandled error; check `wrangler tail` |
| 503 | `oauth_not_configured` | OAuth App credentials missing on the Worker |

## On the Rust client side

`forage_hub::HubClient` decodes the envelope on every non-2xx
response:

```rust
match client.publish(slug, body, meta).await {
    Err(HubError::Api { status: 429, code, message }) if code == "rate_limited" => {
        // retry with backoff
    }
    Err(HubError::Api { status: 403, .. }) => {
        // ownership rejection — tell the user to sign in differently
    }
    Err(e) => return Err(e.into()),
    Ok(meta) => println!("published v{}", meta.version),
}
```

## On the TS client side

`hub-site/forage-wasm/adapter.ts` presents:

```ts
catch (e) {
    if (e.code === 'forbidden') { /* ... */ }
    if (e.code === 'rate_limited') {
        await sleep(e.retryAfter * 1000);
        return retry();
    }
}
```

## On the CLI

`forage publish` surfaces the code + message directly:

```text
$ forage publish recipes/foo --publish
hub HTTP error 403: forbidden — recipe alice/foo is owned by alice
```

Exit codes follow the table:

- `0` on success.
- `2` for client-side errors (parse, validate before send).
- `5` for `4xx` HTTP errors from the hub.
- `6` for `5xx` HTTP errors from the hub.
