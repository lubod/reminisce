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
    interface PoolPoint {
        hash: string;
        lon: number;
        lat: number;
        created_at: string;
        place: string | null;
        starred: boolean;
        device_id: string | null;
        has_thumbnail: boolean;
    }
    const mapPoint = (hash: string, i: number): PoolPoint => ({
        hash,
        lon: 14.0 + i,
        lat: 50.0 + i,
        created_at: "2024-01-15T10:00:00Z",
        place: null,
        starred: false,
        device_id: null,
        has_thumbnail: true,
    });
    const state: {
        mapParams: string;
        mapPool: PoolPoint[];
        mapPoint: (hash: string, i: number) => PoolPoint;
        imagesPool: Array<Record<string, unknown>>;
        imagesTotal: number;
        rejectPost: boolean;
    } = {
        mapParams: "",
        mapPool: [mapPoint("m1", 0)],
        mapPoint,
        imagesPool: [],
        imagesTotal: 0,
        rejectPost: false,
    };
    return {
        state,
        post: async () => {
            if (state.rejectPost) throw new Error("server failure");
            return { data: { starred: true }, status: 200 };
        },
        get: async (url: string) => {
            const u = new URL(url, "http://localhost");
            if (url.includes("/map/media")) {
                state.mapParams = u.searchParams.toString();
                const limit = Number(u.searchParams.get("limit") || 10000);
                // Keyset cursor: filter to strictly newer? No — the endpoint is ordered
                // created_at DESC, so the cursor means "strictly older than (after_created_at,
                // after_hash)". Emulate by slicing the pool (already newest-first) from the
                // first index whose point sorts after the cursor.
                const afterCreatedAt = u.searchParams.get("after_created_at");
                const afterHash = u.searchParams.get("after_hash");
                const sorted = [...state.mapPool].sort((a, b) =>
                    a.created_at === b.created_at
                        ? (a.hash < b.hash ? 1 : -1)
                        : (a.created_at < b.created_at ? 1 : -1)
                );
                let start = 0;
                if (afterCreatedAt && afterHash) {
                    start = sorted.findIndex((p) =>
                        p.created_at < afterCreatedAt ||
                        (p.created_at === afterCreatedAt && p.hash < afterHash)
                    );
                    if (start < 0) start = sorted.length;
                }
                return {
                    data: {
                        points: sorted.slice(start, start + limit),
                        total: state.mapPool.length,
                    },
                    status: 200,
                };
            }
            if (url.includes("/search/images")) {
                const offset = Number(u.searchParams.get("offset") || 0);
                const limit = Number(u.searchParams.get("limit") || 50);
                return { data: { results: pool.slice(offset, offset + limit), total: pool.length }, status: 200 };
            }
            if (url.includes("/image_thumbnails")) {
                const page = Number(u.searchParams.get("page") || 1);
                const limit = Number(u.searchParams.get("limit") || 50);
                const start = (page - 1) * limit;
                return {
                    data: {
                        thumbnails: state.imagesPool.slice(start, start + limit),
                        total: state.imagesTotal,
                        page,
                        limit,
                    },
                    status: 200,
                };
            }
            return { data: { bytes: 1 }, status: 200 };
        },
    };
});

vi.mock("../api/axiosConfig", () => ({
    __esModule: true,
    default: { get: api.get, post: api.post },
}));

// MediaStore's computed/pure logic (filtering, grouping, lightbox getters) is
// tested without any network: the MobX filter reaction is debounced (400ms) and
// never fires during these synchronous tests, so applyFilters/performSearch
// (which would hit the API) are not invoked.

function makeStore(): MediaStore {
    const mockRoot = { uiStore: { setError: () => {}, setLoading: () => {} } } as unknown as RootStore;
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


describe("map points (fetchMapPoints)", () => {
    it("maps browse filters to query params and stores points", async () => {
        api.state.mapParams = "";
        const s = makeStore();
        s.filters.starredOnly = true;
        s.filters.startDate = "2024-01-01";
        s.filters.endDate = "2024-02-01";
        s.filters.selectedLabelId = 7;
        s.filters.selectedDeviceId = "devX";
        s.mapActive = true;

        await s.fetchMapPoints();

        expect(api.state.mapParams).toContain("starred_only=true");
        expect(api.state.mapParams).toContain("start_date=2024-01-01");
        expect(api.state.mapParams).toContain("end_date=2024-02-01");
        expect(api.state.mapParams).toContain("label_id=7");
        expect(api.state.mapParams).toContain("device_id=devX");
        expect(s.mapPoints.length).toBe(1);
        expect(s.mapPoints[0].hash).toBe("m1");
        expect(s.mapTotal).toBe(1);
    });

    it("pages through the whole geotagged library (server caps per-page)", async () => {
        api.state.mapPool = Array.from({ length: 12000 }, (_, i) => api.state.mapPoint(`pm${i}`, i));
        const s = makeStore();
        s.mapActive = true;

        await s.fetchMapPoints();

        expect(s.mapPoints.length).toBe(12000);
        expect(s.mapTotal).toBe(12000);
        // Pagination advances via the keyset cursor (latest page request carried
        // after_created_at + after_hash rather than an OFFSET page number).
        const url = new URL("http://localhost?" + api.state.mapParams);
        expect(url.searchParams.get("page")).toBeNull();
        expect(url.searchParams.get("after_created_at")).not.toBeNull();
        expect(url.searchParams.get("after_hash")).not.toBeNull();
    });
});


describe("map auto-fetch + star/delete + fetchImages", () => {
    it("setMapActive(true) auto-fetches map points", async () => {
        api.state.mapParams = "";
        api.state.mapPool = [api.state.mapPoint("m1", 0)];
        const s = makeStore();
        expect(s.mapPoints).toHaveLength(0);
        s.setMapActive(true);
        await new Promise(r => setTimeout(r, 0));
        expect(api.state.mapParams).toContain("page=1");
        expect(s.mapPoints).toHaveLength(1);
        expect(s.mapTotal).toBe(1);
    });

    it("setMapActive does not fetch in search mode", async () => {
        const s = makeStore();
        s.searchMode = true;
        s.setMapActive(true);
        expect(s.mapPoints).toHaveLength(0);
    });

    it("toggleStarMedia flips starred across arrays and applies the server value", async () => {
        const s = makeStore();
        const target = item({ device_id: "a", starred: false, media_type: "image" });
        s.images = [target];
        s.allMedia = [target];

        await s.toggleStarMedia(target.hash, target.device_id);

        expect(s.images[0].starred).toBe(true);
        expect(s.allMedia[0].starred).toBe(true);
    });

    it("toggleStarMedia matches on hash AND device_id", async () => {
        const s = makeStore();
        const mine = item({ hash: "dup", device_id: "devA", starred: false, media_type: "image" });
        const other = item({ hash: "dup", device_id: "devB", starred: false, media_type: "image" });
        s.images = [mine];
        s.allMedia = [other];

        await s.toggleStarMedia(mine.hash, mine.device_id);

        expect(s.images[0].starred).toBe(true);
        // Same hash but different device must stay untouched.
        expect(s.allMedia[0].starred).toBe(false);
    });

    it("toggleStarMedia rolls back to the previous value on server failure", async () => {
        const setError = vi.fn();
        const s = new MediaStore({ uiStore: { setError, setLoading: () => {} } } as unknown as RootStore);
        const target = item({ device_id: "a", starred: false, media_type: "image" });
        s.images = [target];

        api.state.rejectPost = true;
        try {
            await s.toggleStarMedia(target.hash, target.device_id);
        } finally {
            api.state.rejectPost = false;
        }

        expect(s.images[0].starred).toBe(false);
        expect(setError).toHaveBeenCalled();
    });

    it("deleteMedia removes the item from every list", async () => {
        const s = makeStore();
        const a = item({ media_type: "image" });
        const b = item({ media_type: "image" });
        const c = item({ media_type: "video" });
        s.images = [a, b];
        s.videos = [c];
        s.allMedia = [a, b, c];

        await s.deleteMedia(b.hash);

        expect(s.images.map(i => i.hash)).toEqual([a.hash]);
        expect(s.videos.map(i => i.hash)).toEqual([c.hash]);
        expect(s.allMedia.map(i => i.hash)).toEqual([a.hash, c.hash]);
    });

    it("deleteMedia clamps the lightbox index when the selected last item is deleted", async () => {
        const s = makeStore();
        const a = item({ media_type: "image" });
        const b = item({ media_type: "image" });
        s.allMedia = [a, b];
        s.lightboxSource = "all";
        s.selectedMediaIndex = 1;
        await s.deleteMedia(b.hash);
        expect(s.allMedia.map(i => i.hash)).toEqual([a.hash]);
        expect(s.selectedMediaIndex).toBe(0);
    });

    it("fetchImages sets hasMore from the server total and pages", async () => {
        api.state.imagesTotal = 3;
        api.state.imagesPool = ["x1", "x2", "x3"].map(h => ({ hash: h, name: h }));
        const s = new MediaStore({ uiStore: { setLoading: () => {}, setError: () => {} } } as unknown as RootStore);

        await s.fetchImages(1, 2);
        expect(s.images).toHaveLength(2);
        expect(s.hasMore).toBe(true);

        await s.fetchImages(2, 2, true);
        expect(s.images).toHaveLength(3);
        expect(s.hasMore).toBe(false);
    });
});

