# Inputs and secrets

A recipe doesn't hardcode store identifiers, page sizes, or credentials.
Two declaration forms let the consumer plug those in at run time:

## `input`

```forage
input storeId:        String
input priceCategoryIds: [Int]
input menuTypes:      [MenuType]
input siteOrigin:     String        // e.g. "https://remedymaryland.com"
```

Inputs become available as `$input.<name>` everywhere in the recipe
body, including templates:

```forage
url "https://api.example.com/items?store={$input.storeId}"
```

The CLI takes an explicit `--inputs <path>` flag pointing at a JSON
object of bindings:

```json
{
    "storeId": "577",
    "priceCategoryIds": [5687, 5685, 5686],
    "menuTypes": ["RECREATIONAL", "MEDICAL"],
    "siteOrigin": "https://remedymaryland.com"
}
```

```sh
forage run remedy-maryland --inputs ./remedy-inputs.json
```

Forage Studio binds inputs through its UI; the web IDE accepts them in
the Run panel.

`input` declarations are recipe-local — they don't take `share` and
they only appear in files that declare a recipe header.

Inputs are typed. `input storeId: String` and a JSON value `577` (a
number) trigger a coercion error before any step runs.

## `secret`

```forage
secret username
secret password
secret apiToken
```

`$secret.<name>` is resolved at run time by the host, **never** carried
in the recipe text. Resolution rules:

| Host           | Source                                       |
|----------------|----------------------------------------------|
| CLI            | `FORAGE_SECRET_<NAME>` env vars              |
| Forage Studio  | macOS Keychain (`com.foragelang.studio`)     |
| Web IDE        | not resolved — sessioned recipes refuse to run |

Secrets compose with templates the same way inputs do:

```forage
auth.staticHeader {
    name:  "Authorization"
    value: "Bearer {$secret.apiToken}"
}
```

The validator flags unreferenced secrets and undeclared
`$secret.<name>` references at parse time, before any HTTP traffic
fires.

## Naming

By convention inputs use `camelCase`; secrets use lowercase (`apiToken`,
not `API_TOKEN`). The env-var resolver upper-cases automatically:
`FORAGE_SECRET_APITOKEN` matches `$secret.apiToken`.
