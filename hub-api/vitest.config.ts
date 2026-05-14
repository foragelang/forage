import { defineConfig } from 'vitest/config'
import { cloudflareTest } from '@cloudflare/vitest-pool-workers'

// Each test file runs inside a real Miniflare worker context. Storage
// (KV, R2) is isolated between test files by default, so individual
// tests don't need teardown.
export default defineConfig({
    plugins: [
        cloudflareTest({
            wrangler: { configPath: './wrangler.toml' },
            miniflare: {
                // Stub the secret bindings declared in wrangler.toml.
                // Tests sign their own JWTs against `JWT_SIGNING_KEY`
                // to act as different users.
                bindings: {
                    HUB_PUBLISH_TOKEN: 'test-admin-token',
                    JWT_SIGNING_KEY: 'test-jwt-signing-key',
                    GITHUB_CLIENT_ID: 'test-gh-client-id',
                    GITHUB_CLIENT_SECRET: 'test-gh-client-secret',
                },
            },
        }),
    ],
})
