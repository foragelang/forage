import type {
    Env,
    ForkRequest,
    PackageMetadata,
    PackageVersion,
} from '../types'
import {
    getPackage,
    putPackage,
    getVersion,
    putVersion,
    indexAddPackage,
    indexAddUserPackage,
    indexAddCategory,
} from '../storage'
import { identifyCaller } from '../auth'
import { json, jsonError } from '../http'
import { validateSegment, newForkedFrom } from './packages'

// `POST /v1/packages/:upstreamAuthor/:upstreamSlug/fork`
//
// Body: `{ "as": "fork-slug" | null }`. Creates `@me/:as` (defaulting
// to `:upstreamSlug`) with v1 carrying the upstream's full content +
// `forked_from` lineage. Bumps upstream's `fork_count` + `downloads`.
//
// Materialized-copy semantics: the fork's v1 is a full snapshot of
// the upstream's latest version (recipe + decls + fixtures +
// snapshot). For 20 MiB R2-backed artifacts that means a full R2
// fetch + KV write at fork time. The plan picks this over a "lazy"
// `forked_from`-only fork so the fork is self-contained and diverges
// cleanly from the upstream — no implicit content tracking, no
// dependency on the upstream version remaining reachable.
//
// One-shot lineage: the fork is independent after creation. There is
// no auto-tracking and `forked_from` is not updated on subsequent
// publishes against the fork.
export async function createFork(
    request: Request,
    env: Env,
    upstreamAuthor: string,
    upstreamSlug: string,
): Promise<Response> {
    const caller = await identifyCaller(request, env)
    if (caller === null || caller.kind !== 'user') {
        return jsonError(401, 'unauthorized', 'sign-in required to fork', {}, request)
    }

    let body: ForkRequest
    try {
        body = (await request.json()) as ForkRequest
    } catch {
        return jsonError(400, 'bad_json', 'request body is not valid JSON', {}, request)
    }
    if (
        body === null
        || typeof body !== 'object'
        || (body.as !== null && typeof body.as !== 'string')
    ) {
        return jsonError(400, 'bad_request', 'body must be { "as": string | null }', {}, request)
    }

    const upstreamMeta = await getPackage(env, upstreamAuthor, upstreamSlug)
    if (upstreamMeta === null) {
        return jsonError(
            404,
            'not_found',
            `unknown package: ${upstreamAuthor}/${upstreamSlug}`,
            {},
            request,
        )
    }
    const upstreamArtifact = await getVersion(
        env,
        upstreamAuthor,
        upstreamSlug,
        upstreamMeta.latest_version,
    )
    if (upstreamArtifact === null) {
        return jsonError(
            500,
            'corrupt',
            `upstream ${upstreamAuthor}/${upstreamSlug} has metadata but no v${upstreamMeta.latest_version}`,
            {},
            request,
        )
    }

    const forkSlug = body.as ?? upstreamSlug
    if (!validateSegment(forkSlug)) {
        return jsonError(400, 'bad_slug', `invalid slug: ${forkSlug}`, {}, request)
    }
    if (caller.login === upstreamAuthor && forkSlug === upstreamSlug) {
        return jsonError(
            409,
            'self_fork',
            `cannot fork ${upstreamAuthor}/${upstreamSlug} onto itself`,
            {},
            request,
        )
    }
    const existing = await getPackage(env, caller.login, forkSlug)
    if (existing !== null) {
        return jsonError(
            409,
            'already_exists',
            `${caller.login}/${forkSlug} already exists`,
            {},
            request,
        )
    }

    const now = Date.now()
    const artifact: PackageVersion = {
        author: caller.login,
        slug: forkSlug,
        version: 1,
        recipe: upstreamArtifact.recipe,
        decls: upstreamArtifact.decls,
        fixtures: upstreamArtifact.fixtures,
        snapshot: upstreamArtifact.snapshot,
        base_version: null,
        published_at: now,
        published_by: caller.login,
    }
    const meta: PackageMetadata = {
        author: caller.login,
        slug: forkSlug,
        description: upstreamMeta.description,
        category: upstreamMeta.category,
        tags: upstreamMeta.tags,
        forked_from: newForkedFrom(
            upstreamAuthor,
            upstreamSlug,
            upstreamMeta.latest_version,
        ),
        created_at: now,
        latest_version: 1,
        stars: 0,
        downloads: 0,
        fork_count: 0,
        owner_login: caller.login,
    }

    await putVersion(env, artifact)
    await putPackage(env, meta)
    await indexAddPackage(env, caller.login, forkSlug)
    await indexAddUserPackage(env, caller.login, forkSlug)
    await indexAddCategory(env, meta.category, caller.login, forkSlug)

    // Bump upstream counters. Order doesn't matter; both are
    // non-transactional.
    upstreamMeta.fork_count += 1
    upstreamMeta.downloads += 1
    await putPackage(env, upstreamMeta)

    return json(meta, 201, request)
}
