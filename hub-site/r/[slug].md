---
title: "{{ $params.displayName }}"
---

# {{ $params.displayName }}

{{ $params.summary }}

<div class="recipe-meta">
  <span v-if="$params.author"><strong>Author:</strong> {{ $params.author }}</span>
  <span v-if="$params.platform"><strong>Platform:</strong> {{ $params.platform }}</span>
  <span><strong>Version:</strong> {{ $params.version }}</span>
  <span class="sha"><strong>sha256:</strong> <code>{{ $params.sha256.slice(0, 12) }}…</code></span>
</div>

<div v-if="$params.tags" class="recipe-tags">
  <span v-for="t in $params.tags.split(',').map(s => s.trim()).filter(Boolean)" :key="t" class="recipe-tag">{{ t }}</span>
</div>

## Source

<!-- @content -->

## Use it from a recipe

```forage
import {{ $params.slug }}
```

## Edit on web

[/r/{{ $params.slug }}/edit](/r/{{ $params.slug }}/edit) — open in the browser-based IDE.

## Open in Studio

[forage-studio://recipe/{{ $params.slug }}](forage-studio://recipe/{{ $params.slug }})

## Raw

- [Source]({{ $params.apiBase }}/v1/packages/{{ $params.slug }})
- [Versions]({{ $params.apiBase }}/v1/packages/{{ $params.slug }}/versions)
- [Fixtures]({{ $params.apiBase }}/v1/packages/{{ $params.slug }}/fixtures) (if present)
- [Snapshot]({{ $params.apiBase }}/v1/packages/{{ $params.slug }}/snapshot) (if present)
