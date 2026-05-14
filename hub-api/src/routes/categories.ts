import type { Env } from '../types'
import { listCategoriesIndex } from '../storage'
import { json } from '../http'

// `GET /v1/categories`
export async function listCategories(
    request: Request,
    env: Env,
): Promise<Response> {
    const items = await listCategoriesIndex(env)
    return json({ items }, 200, request)
}
