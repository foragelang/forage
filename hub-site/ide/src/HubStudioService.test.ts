//! Unit tests for `HubStudioService`'s fetch + capability surface.
//!
//! The WASM-driven methods (validate/run) aren't exercised here — the
//! Rust-side `forage-wasm` crate has its own parity test in
//! `crates/forage-wasm/tests/replay.rs`. These tests pin the wire
//! mapping (URL shapes, 409 → StaleBaseError) and the capability gates
//! that drive the UI's "what's possible here" branches.

import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

import { StaleBaseError, NotSupportedByService } from "@/lib/services";

import { HubStudioService } from "./HubStudioService";

const HUB = "https://api.example.test";

function jsonResponse(body: unknown, status = 200): Response {
    return new Response(JSON.stringify(body), {
        status,
        headers: { "content-type": "application/json" },
    });
}

describe("HubStudioService", () => {
    let fetchMock: ReturnType<typeof vi.fn>;

    beforeEach(() => {
        fetchMock = vi.fn();
        (globalThis as unknown as { fetch: typeof fetch }).fetch = fetchMock as unknown as typeof fetch;
    });
    afterEach(() => {
        vi.restoreAllMocks();
    });

    test("listPackages encodes the sort/category/q query params", async () => {
        fetchMock.mockResolvedValueOnce(jsonResponse({ items: [], next_cursor: null }));
        const svc = new HubStudioService(HUB);
        await svc.listPackages({ sort: "top_starred", category: "dispensary", q: "weed", limit: 20 });
        expect(fetchMock).toHaveBeenCalledTimes(1);
        const url = fetchMock.mock.calls[0]![0] as string;
        expect(url).toBe(
            `${HUB}/v1/packages?sort=top_starred&category=dispensary&q=weed&limit=20`,
        );
    });

    test("getPackage URL-encodes author + slug", async () => {
        fetchMock.mockResolvedValueOnce(
            jsonResponse({
                author: "al ice",
                slug: "zen leaf",
                description: "",
                category: "",
                tags: [],
                forked_from: null,
                created_at: 0,
                latest_version: 1,
                stars: 0,
                downloads: 0,
                fork_count: 0,
                owner_login: "al ice",
            }),
        );
        const svc = new HubStudioService(HUB);
        await svc.getPackage("al ice", "zen leaf");
        const url = fetchMock.mock.calls[0]![0] as string;
        expect(url).toBe(`${HUB}/v1/packages/al%20ice/zen%20leaf`);
    });

    test("publishVersion translates 409 into a StaleBaseError", async () => {
        fetchMock.mockResolvedValueOnce(
            new Response(
                JSON.stringify({
                    error: {
                        code: "stale_base",
                        message: "base is stale, rebase to v3 and retry",
                        latest_version: 3,
                        your_base: 1,
                    },
                }),
                {
                    status: 409,
                    headers: { "content-type": "application/json" },
                },
            ),
        );
        const svc = new HubStudioService(HUB);
        const err = await svc
            .publishVersion("alice", "zen-leaf", {
                description: "",
                category: "",
                tags: [],
                recipe: "",
                decls: [],
                fixtures: [],
                snapshot: null,
                base_version: 1,
            })
            .catch((e) => e as unknown);
        expect(err).toBeInstanceOf(StaleBaseError);
        const sbe = err as StaleBaseError;
        expect(sbe.latestVersion).toBe(3);
        expect(sbe.yourBase).toBe(1);
    });

    test("starPackage sends credentials", async () => {
        fetchMock.mockResolvedValueOnce(new Response(null, { status: 204 }));
        const svc = new HubStudioService(HUB);
        await svc.starPackage("alice", "zen-leaf");
        const init = fetchMock.mock.calls[0]![1] as RequestInit;
        expect(init.method).toBe("POST");
        expect(init.credentials).toBe("include");
    });

    test("forkPackage POSTs the `as` slug body", async () => {
        fetchMock.mockResolvedValueOnce(
            jsonResponse({
                author: "me",
                slug: "my-fork",
                description: "",
                category: "",
                tags: [],
                forked_from: { author: "alice", slug: "zen-leaf", version: 3 },
                created_at: 0,
                latest_version: 1,
                stars: 0,
                downloads: 0,
                fork_count: 0,
                owner_login: "me",
            }),
        );
        const svc = new HubStudioService(HUB);
        await svc.forkPackage("alice", "zen-leaf", "my-fork");
        const init = fetchMock.mock.calls[0]![1] as RequestInit;
        expect(init.method).toBe("POST");
        expect(JSON.parse(String(init.body))).toEqual({ as: "my-fork" });
    });

    test("workspace methods throw NotSupportedByService", async () => {
        const svc = new HubStudioService(HUB);
        await expect(svc.openWorkspace()).rejects.toBeInstanceOf(NotSupportedByService);
        await expect(svc.newWorkspace()).rejects.toBeInstanceOf(NotSupportedByService);
        await expect(svc.closeWorkspace()).rejects.toBeInstanceOf(NotSupportedByService);
        await expect(svc.createRecipe()).rejects.toBeInstanceOf(NotSupportedByService);
        await expect(svc.configureRun()).rejects.toBeInstanceOf(NotSupportedByService);
        await expect(svc.triggerRun()).rejects.toBeInstanceOf(NotSupportedByService);
    });

    test("capabilities advertise hub-shape (no workspace, no deploy, no live)", () => {
        const svc = new HubStudioService(HUB);
        expect(svc.capabilities).toEqual({
            workspace: false,
            deploy: false,
            liveRun: false,
            hubPackages: true,
        });
    });

    test("runRecipe(live) is rejected — the hub has no transport", async () => {
        const svc = new HubStudioService(HUB);
        await expect(svc.runRecipe("any", false)).rejects.toBeInstanceOf(
            NotSupportedByService,
        );
    });

    test("authWhoami returns the login when signed in", async () => {
        fetchMock.mockResolvedValueOnce(
            jsonResponse({
                authenticated: true,
                user: { login: "alice", name: "Alice", avatarUrl: "https://x/y" },
            }),
        );
        const svc = new HubStudioService(HUB);
        const login = await svc.authWhoami();
        expect(login).toBe("alice");
        const [url, init] = fetchMock.mock.calls[0]!;
        expect(url).toBe(`${HUB}/v1/oauth/whoami`);
        expect((init as RequestInit).credentials).toBe("include");
    });

    test("authWhoami returns null when not signed in", async () => {
        fetchMock.mockResolvedValueOnce(jsonResponse({ authenticated: false }));
        const svc = new HubStudioService(HUB);
        expect(await svc.authWhoami()).toBeNull();
    });

    test("authWhoami returns null on non-2xx responses", async () => {
        fetchMock.mockResolvedValueOnce(new Response(null, { status: 500 }));
        const svc = new HubStudioService(HUB);
        expect(await svc.authWhoami()).toBeNull();
    });
});
