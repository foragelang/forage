---
layout: page
sidebar: false
title: "Producers of {{ $params.author }}/{{ $params.name }}"
---

<DiscoverByType kind="producers" :type-author="$params.author" :type-name="$params.name" />
