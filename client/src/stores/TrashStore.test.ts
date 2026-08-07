import { describe, it, expect, vi } from "vitest";
import { TrashStore, type TrashItem } from "./TrashStore";
import type { RootStore } from "./RootStore";

const mocks = vi.hoisted(() => {
    const routes: Record<string, () => { data: unknown }> = {};
    const get = vi.fn(async (url: string) => {
        const fn = routes[url];
        if (!fn) return { data: [], status: 200 };
        return fn();
    });
    const post = vi.fn(async (url: string, body: unknown) => { void url; void body; return { data: {}, status: 200 }; });
    const clear = () => { for (const k of Object.keys(routes)) delete routes[k]; };
    return { api: { get, post }, routes, clear };
});

vi.mock("../api/axiosConfig", () => ({ __esModule: true, default: mocks.api }));

const items: TrashItem[] = [
    { hash: "h1", name: "a.jpg", created_at: "2024-01-01T00:00:00Z", ext: "jpg", type: "", deviceid: null, deleted_at: "2024-02-01T00:00:00Z", media_type: "image" },
    { hash: "h2", name: "b.mp4", created_at: "2024-01-02T00:00:00Z", ext: "mp4", type: "", deviceid: null, deleted_at: "2024-02-02T00:00:00Z", media_type: "video" },
];

function makeStore(): TrashStore {
    const mockRoot = { uiStore: { setError: () => {} } } as unknown as RootStore;
    return new TrashStore(mockRoot);
}

describe("TrashStore", () => {
    it("getThumbnailUrl is cookie-authenticated", () => {
        const s = makeStore();
        expect(s.getThumbnailUrl(items[0])).toBe("/api/thumbnail/h1");
    });

    it("fetchTrash populates items", async () => {
        mocks.routes["/trash"] = () => ({ data: items });
        const s = makeStore();
        await s.fetchTrash();
        expect(s.items).toHaveLength(2);
        expect(s.items[0].hash).toBe("h1");
        expect(s.isLoading).toBe(false);
        expect(s.error).toBeNull();
    });

    it("fetchTrash failure sets an error and keeps loading false", async () => {
        mocks.routes["/trash"] = () => { throw new Error("network"); };
        const s = makeStore();
        await s.fetchTrash();
        expect(s.error).toBe("Failed to load trash");
        expect(s.isLoading).toBe(false);
        expect(s.items).toHaveLength(0);
    });

    it("restoreItem removes the item from the list", async () => {
        mocks.routes["/trash"] = () => ({ data: items });
        const s = makeStore();
        await s.fetchTrash();

        mocks.api.post.mockImplementationOnce(async () => ({ data: {}, status: 200 }));
        await s.restoreItem("h1", "image");
        expect(s.items.map(i => i.hash)).toEqual(["h2"]);
    });

    it("restoreItem failure keeps the item and sets an error", async () => {
        mocks.routes["/trash"] = () => ({ data: items });
        const s = makeStore();
        await s.fetchTrash();

        mocks.api.post.mockImplementationOnce(async () => { throw new Error("boom"); });
        await s.restoreItem("h1", "image");
        expect(s.items).toHaveLength(2);
        expect(s.error).toBe("Failed to restore item");
    });

    it("restoreItem posts to the right media_type endpoint", async () => {
        mocks.routes["/trash"] = () => ({ data: items });
        const s = makeStore();
        await s.fetchTrash();
        await s.restoreItem("h2", "video");
        expect(mocks.api.post).toHaveBeenCalledWith("/video/h2/restore");
    });
});
