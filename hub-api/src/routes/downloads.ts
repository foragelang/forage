import type { Env } from '../types'
import { getPackage, putPackage } from '../storage'
import { json, jsonError } from '../http'

// `POST /v1/packages/:author/:slug/downloads`
//
// Increments the package's download counter by one. Called by Studio's
// sync_from_hub and by the fork endpoint; the in-browser IDE doesn't
// hit this because read-only views aren't downloads. No auth — the
// counter is purely informational and we don't want a sign-in gate
// between Studio's `forage sync` and the count.
export async function recordDownload(
    request: Request,
    env: Env,
    author: string,
    slug: string,
): Promise<Response> {
    const meta = await getPackage(env, author, slug)
    if (meta === null) {
        return jsonError(404, 'not_found', `unknown package: ${author}/${slug}`, {}, request)
    }
    meta.downloads += 1
    await putPackage(env, meta)
    return json({ downloads: meta.downloads }, 200, request)
}
