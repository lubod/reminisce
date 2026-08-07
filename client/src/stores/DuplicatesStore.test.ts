import { describe, it, expect, vi, beforeEach } from "vitest";
import { DuplicatesStore } from "./DuplicatesStore";
import type { RootStore } from "./RootStore";

const mocks = vi.hoisted(() => {
    const routes: Record<string, () => { data: unknown }> = {};
    const get = vi.fn(async (url: string) => {
        const fn = routes[url];
        if (!fn) return { data: { groups: [], total_groups: 0, page: 1, limit: 20 }, status: 200 };
        return fn();
    });
    const post = vi.fn(async (url: string, body: unknown) => { void url; void body; return { data: {}, status: 200 }; });
    const clear = () => { for (const k of Object.keys(routes)) delete routes[k]; };
    return { api: { get, post }, routes, clear };
});

vi.mock("../api/axiosConfig", () => ({ __esModule: true, default: mocks.api }));

function makeStore(): DuplicatesStore {
    const mockRoot = { uiStore: { setError: () => {}, setSuccess: () => {} } } as unknown as RootStore;
    return new DuplicatesStore(mockRoot);
}

function group(hashes: string[], sim = 0.99) {
    return {
        similarity: sim,
        images: hashes.map(hash => ({
            hash,
            name: `${hash}.jpg`,
            created_at: "2024-01-01T00:00:00Z",
            thumbnail_url: `/api/thumbnail/${hash}`,
            aesthetic_score: null,
            sharpness_score: null,
            width: null,
            height: null,
            file_size_bytes: null,
        })),
    };
}

describe("DuplicatesStore", () => {
    beforeEach(() => mocks.clear());

    it("setThreshold clamps to 0.8..1.0", () => {
        const s = makeStore();
        s.setThreshold(0.5);
        expect(s.threshold).toBe(0.8);
        s.setThreshold(1.5);
        expect(s.threshold).toBe(1.0);
        s.setThreshold(0.92);
        expect(s.threshold).toBe(0.92);
    });

    it("hasMore reflects totalGroups", () => {
        const s = makeStore();
        expect(s.hasMore).toBe(false);
        s.totalGroups = 5;
        s.groups = [group(["a", "b"])];
        expect(s.hasMore).toBe(true);
    });

    it("fetchDuplicates fetches page 1 with the threshold and resets state", async () => {
        mocks.routes["/duplicates"] = () => ({ data: { groups: [group(["a", "b"])], total_groups: 3, page: 1, limit: 20 } });
        const s = makeStore();
        s.page = 9;
        s.threshold = 0.9;
        await s.fetchDuplicates();

        expect(mocks.api.get).toHaveBeenCalledWith("/duplicates", {
            params: { threshold: 0.9, page: 1, limit: 20 },
        });
        expect(s.groups).toHaveLength(1);
        expect(s.totalGroups).toBe(3);
        expect(s.page).toBe(1);
        expect(s.hasMore).toBe(true);
        expect(s.isLoading).toBe(false);
    });

    it("fetchDuplicates failure sets an error", async () => {
        mocks.routes["/duplicates"] = () => { throw new Error("x"); };
        const s = makeStore();
        await s.fetchDuplicates();
        expect(s.error).toBe("Failed to load duplicates");
        expect(s.isLoading).toBe(false);
    });

    it("loadNextPage appends the next page and advances page", async () => {
        const calls = mocks.api.get as ReturnType<typeof vi.fn>;
        calls.mockImplementationOnce(async () => ({ data: { groups: [group(["a", "b"])], total_groups: 2, page: 1, limit: 1 }, status: 200 }))
            .mockImplementationOnce(async () => ({ data: { groups: [group(["c", "d"])], total_groups: 2, page: 2, limit: 1 }, status: 200 }));

        const s = makeStore();
        await s.fetchDuplicates();
        await s.loadNextPage();

        expect(s.groups).toHaveLength(2);
        expect(s.page).toBe(2);
        expect(s.hasMore).toBe(false);
        expect(s.isLoadingMore).toBe(false);
    });

    it("loadNextPage does nothing when there is no next page", async () => {
        mocks.routes["/duplicates"] = () => ({ data: { groups: [group(["a", "b"])], total_groups: 1, page: 1, limit: 20 } });
        const s = makeStore();
        await s.fetchDuplicates();
        const getCallsBefore = mocks.api.get.mock.calls.length;
        await s.loadNextPage();
        expect(mocks.api.get.mock.calls.length).toBe(getCallsBefore);
        expect(s.page).toBe(1);
    });

    it("deleteImage removes the image from groups and drops groups below size 2", async () => {
        mocks.routes["/duplicates"] = () => ({ data: { groups: [group(["a", "b"]), group(["c", "d", "e"])], total_groups: 2, page: 1, limit: 20 } });
        const s = makeStore();
        await s.fetchDuplicates();
        expect(s.groups).toHaveLength(2);

        await s.deleteImage("a");
        expect(s.groups).toHaveLength(1); // a+b became single -> dropped
        expect(s.totalGroups).toBe(1);
        expect(s.groups[0].images.map(i => i.hash)).toEqual(["c", "d", "e"]);

        mocks.api.post.mockImplementationOnce(async () => { throw new Error("x"); });
        await s.deleteImage("d");
        expect(s.error).toBe("Failed to delete image");
    });
});
