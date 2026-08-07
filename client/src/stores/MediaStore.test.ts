import { describe, it, expect, vi, beforeEach } from "vitest";
import { MediaStore, type MediaItem } from "./MediaStore";
import type { RootStore } from "./RootStore";

// Search pagination tests need a mocked API: a pool of 120 results served in
// pages, `/thumbnail/{hash}` returning anything (URL.createObjectURL is a no-op
// in the Node test env and falls back to returning the item unharmed).
const api = vi.hoisted(() => {
    const pool = Array.from({ length: 120 }, (_, i) => ({
        hash: `h${i}`,
        name: `n${i}.jpg`,
        created_at: "2024-01-15T10:00:00Z",
    }));
    return {
        get: async (url: string) => {
            const u = new URL(url, "http://localhost");
            if (url.includes("/search/images")) {
                const offset = Number(u.searchParams.get("offset") || 0);
                const limit = Number(u.searchParams.get("limit") || 50);
                return { data: { results: pool.slice(offset, offset + limit), total: pool.length }, status: 200 };
            }
            return { data: { bytes: 1 }, status: 200 };
        },
    };
});

vi.mock("../api/axiosConfig", () => ({
    __esModule: true,
    default: { get: api.get },
}));

// MediaStore's computed/pure logic (filtering, grouping, lightbox getters) is
// tested without any network: the MobX filter reaction is debounced (400ms) and
// never fires during these synchronous tests, so applyFilters/performSearch
// (which would hit the API) are not invoked.

function makeStore(): MediaStore {
    const mockRoot = { uiStore: { setError: () => {} } } as unknown as RootStore;
    return new MediaStore(mockRoot);
}

let counter = 0;
function item(over: Partial<MediaItem>): MediaItem {
    counter += 1;
    return {
        hash: `hash_${counter}`,
        name: `name_${counter}`,
        created_at: "2024-01-15T10:00:00Z",
        ...over,
    };
}

describe("filtered* (device filter)", () => {
    it("returns all items when device filter is 'all'", () => {
        const s = makeStore();
        s.images = [item({ device_id: "a" }), item({ device_id: "b" }), item({ device_id: "a" })];
        s.filters.selectedDeviceId = "all";
        expect(s.filteredImages).toHaveLength(3);
    });

    it("filters items by selected device id", () => {
        const s = makeStore();
        s.images = [item({ device_id: "a" }), item({ device_id: "b" }), item({ device_id: "a" })];
        s.videos = [item({ device_id: "a" }), item({ device_id: "c" })];
        s.allMedia = [...s.images, ...s.videos];
        s.filters.selectedDeviceId = "a";

        expect(s.filteredImages.map(i => i.device_id)).toEqual(["a", "a"]);
        expect(s.filteredVideos.map(i => i.device_id)).toEqual(["a"]);
        expect(s.filteredAllMedia).toHaveLength(3);
    });
});

describe("groupedAllMedia (grouping + sorting)", () => {
    it("groups by day, newest day first by default", () => {
        const s = makeStore();
        s.allMedia = [
            item({ created_at: "2024-01-15T10:00:00Z" }),
            item({ created_at: "2024-01-14T09:00:00Z" }),
            item({ created_at: "2024-01-15T08:00:00Z" }),
        ];
        s.allMediaGroupBy = "day";
        s.sortBy = "date";
        s.sortOrder = "desc";

        const groups = s.groupedAllMedia;
        expect(groups.map(g => g.date)).toEqual(["2024-01-15", "2024-01-14"]);
        // Items within a group are sorted by created_at desc.
        expect(groups[0].items.map(i => i.created_at)).toEqual([
            "2024-01-15T10:00:00Z",
            "2024-01-15T08:00:00Z",
        ]);
        expect(groups[1].items).toHaveLength(1);
    });

    it("groups by place, using 'Unknown Location' for missing place", () => {
        const s = makeStore();
        s.allMedia = [
            item({ place: "Paris" }),
            item({ place: "London" }),
            item({ place: undefined }),
            item({ place: "Paris" }),
        ];
        s.allMediaGroupBy = "place";

        const groups = s.groupedAllMedia;
        const byKey = new Map(groups.map(g => [g.date, g.items.length]));
        expect(byKey.get("Paris")).toBe(2);
        expect(byKey.get("London")).toBe(1);
        expect(byKey.get("Unknown Location")).toBe(1);
    });

    it("sorts groups and items by size when sortBy='size'", () => {
        const s = makeStore();
        s.allMedia = [
            item({ created_at: "2024-01-15T00:00:00Z", file_size_bytes: 100 }),
            item({ created_at: "2024-01-15T01:00:00Z", file_size_bytes: 500 }),
            item({ created_at: "2024-01-14T00:00:00Z", file_size_bytes: 900 }),
        ];
        s.allMediaGroupBy = "day";
        s.sortBy = "size";
        s.sortOrder = "desc";

        const groups = s.groupedAllMedia;
        // Group with the largest max file size (900 on 01-14) comes first.
        expect(groups[0].date).toBe("2024-01-14");
        // Within the 01-15 group, items are sorted by size desc (500 then 100).
        expect(groups[1].items.map(i => i.file_size_bytes)).toEqual([500, 100]);
    });
});

describe("lightbox getters", () => {
    it("activeLightboxItems follows lightboxSource", () => {
        const s = makeStore();
        s.allMedia = [item({}), item({})];
        s.images = [item({})];
        s.customLightboxItems = [item({}), item({}), item({})];

        s.lightboxSource = "all";
        expect(s.activeLightboxItems).toBe(s.allMedia);
        s.lightboxSource = "images";
        expect(s.activeLightboxItems).toBe(s.images);
        s.lightboxSource = "custom";
        expect(s.activeLightboxItems).toBe(s.customLightboxItems);
    });

    it("isFirstMedia / isLastMedia track the selected index", () => {
        const s = makeStore();
        s.allMedia = [item({}), item({}), item({})];
        s.lightboxSource = "all";

        s.selectedMediaIndex = 0;
        expect(s.isFirstMedia).toBe(true);
        expect(s.isLastMedia).toBe(false);

        s.selectedMediaIndex = 2;
        expect(s.isFirstMedia).toBe(false);
        expect(s.isLastMedia).toBe(true);

        s.selectedMediaIndex = 1;
        expect(s.isFirstMedia).toBe(false);
        expect(s.isLastMedia).toBe(false);
    });
});


describe("search pagination (performSearch)", () => {
    beforeEach(() => {
        // reset per-test: construct a fresh store inside each test
    });

    it("loads all pages and flips allMediaHasMore exactly at the end", async () => {
        const s = makeStore();
        s.searchType = "semantic";
        s.filters.allMediaTypeFilter = "all";

        await s.performSearch("sunset");
        expect(s.allMedia).toHaveLength(50);
        expect(s.searchOffset).toBe(50);
        expect(s.totalAllMedia).toBe(120);
        expect(s.allMediaHasMore).toBe(true);

        await s.performSearch("sunset", true);
        expect(s.allMedia).toHaveLength(100);
        expect(s.searchOffset).toBe(100);
        expect(s.allMediaHasMore).toBe(true);

        await s.performSearch("sunset", true);
        expect(s.allMedia).toHaveLength(120);
        expect(s.searchOffset).toBe(120);
        // offset(100) + rows(20) == total(120) -> no more
        expect(s.allMediaHasMore).toBe(false);

        // Stale append past the total must not append duplicate content.
        await s.performSearch("sunset", true);
        expect(s.allMedia).toHaveLength(120);
        expect(s.searchOffset).toBe(120);
        expect(s.allMediaHasMore).toBe(false);
    });
});
