---
title: "Edit {{ $params.displayName || $params.slug }}"
layout: page
sidebar: false
aside: false
---

<RecipeIDE :slug="$params.slug" :api-base="$params.apiBase" />

<style>
.VPDoc.has-sidebar .container {
    max-width: 100% !important;
}
.VPDoc .content-container {
    max-width: 1400px;
}
</style>
