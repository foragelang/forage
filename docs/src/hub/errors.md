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
| 400 | `bad_slug` | `author/slug` segments don't match `^[a-z0-9][a-z0-9-]{0,38}$` |
| 400 | `bad_sort` | Listing sort key is not `recent` / `stars` / `downloads` |
| 400 | `bad_version` | Version path segment is neither a positive integer nor `latest` |
| 400 | `bad_request` | Body shape is wrong (missing required field, etc.) |
| 400 | `invalid` | Publish envelope failed structural validation |
| 400 | `forked_from_on_existing` | `forked_from` set on a v2+ publish; lineage is one-shot |
| 401 | `unauthorized` | Missing or invalid bearer token |
| 403 | `forbidden` | Authenticated but not allowed (publishing under another author, ownership) |
| 404 | `not_found` | Package / version / user doesn't exist |
| 404 | `no_route` | No handler for the path + method |
| 405 | `method_not_allowed` | Verb not supported for this route |
| 409 | `stale_base` | `base_version` doesn't match current `latest_version`; body carries both |
| 409 | `already_exists` | Fork target slug already exists in your namespace |
| 409 | `self_fork` | Tried to fork a package onto its own author/slug |
| 413 | `payload_too_large` | Request envelope > 64 MiB |
| 429 | `rate_limited` | Bucket exceeded; `retryAfter` carries seconds |
| 500 | `corrupt` | Storage inconsistency (e.g. version slot points at missing R2 object) |
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

The hub-site Vue pages branch on `error.code` directly:

```ts
catch (e) {
    if (e.code === 'forbidden') { /* … */ }
    if (e.code === 'stale_base') { /* rebase to e.latest_version and retry */ }
    if (e.code === 'rate_limited') {
        await sleep(e.retryAfter * 1000)
        return retry()
    }
}
```

## On the CLI

`forage publish` surfaces the code + message directly:

```text
$ forage publish ~/Library/Forage/Recipes/foo --publish
hub HTTP error 403: forbidden — recipe alice/foo is owned by alice
```

Exit codes follow the table:

- `0` on success.
- `2` for client-side errors (parse, validate before send).
- `5` for `4xx` HTTP errors from the hub.
- `6` for `5xx` HTTP errors from the hub.
