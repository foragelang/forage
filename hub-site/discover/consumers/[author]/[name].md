---
layout: page
sidebar: false
title: "Consumers of {{ $params.author }}/{{ $params.name }}"
---

<DiscoverByType kind="consumers" :type-author="$params.author" :type-name="$params.name" />
