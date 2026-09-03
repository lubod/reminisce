import { describe, it, expect, vi, beforeEach } from "vitest";
import { configure } from "mobx";
import { MediaStore, type MediaItem } from "./MediaStore";
import type { RootStore } from "./RootStore";

// Keep MobX quiet: direct observable writes in tests would otherwise log
// strict-mode warnings (the store's filter reaction is debounced 400ms and
// never fires during these synchronous tests).
configure({ enforceActions: "never" });

// jsdom/Node do not let objects be treated as blob URLs here; install inert
// overrides so blob-URL creation/revocation is exercised without real objects.
(globalThis as unknown as { URL: { createObjectURL: (o: unknown) => string } }).URL.createObjectURL = () => "blob:mock";
(globalThis as unknown as { URL: { revokeObjectURL: (u: string) => void } }).URL.revokeObjectURL = () => {};

type Handler = { verb: string; re: RegExp; fn: (url: string, u: URL, body?: unknown) => unknown };

const api = vi.hoisted(() => {
    const handlers: Handler[] = [];
    const base = (url: string) => url.split("?")[0];
    const find = (verb: string, url: string) => handlers.find(h => h.verb === verb && h.re.test(base(url)));
    const get = vi.fn(async (url: string) => {
        const h = find("get", url);
        const u = new URL(url, "http://localhost");
        return h ? h.fn(url, u) : { data: { results: [], total: 0, thumbnails: [], page: 1, limit: 50 }, status: 200 };
    });
    const post = vi.fn(async (url: string, body?: unknown) => {
        const h = find("post", url);
        return h ? h.fn(url, new URL(url, "http://localhost"), body) : { data: {}, status: 200 };
    });
    const state: {
        imagePool: MediaItem[];
        videoPool: MediaItem[];
        allPool: MediaItem[];
        noExifPool: MediaItem[];
        totalImage: number;
        totalVideo: number;
        totalAll: number;
        totalNoExif: number;
        searchResults: MediaItem[];
        searchTotal: number;
        mapReject: boolean;
        deviceIds: string[];
        currentUser: { role: string } | null;
        places: unknown[];
        rejectMetadata: boolean;
    } = {
        imagePool: [],
        videoPool: [],
        allPool: [],
        noExifPool: [],
        totalImage: 0,
        totalVideo: 0,
        totalAll: 0,
        totalNoExif: 0,
        searchResults: [],
        searchTotal: 0,
        mapReject: false,
        deviceIds: [],
        currentUser: null,
        places: [],
        rejectMetadata: false,
    };
    return { get, post, handlers, state };
});

vi.mock("../api/axiosConfig", () => ({ __esModule: true, default: { get: api.get, post: api.post } }));

function item(over: Partial<MediaItem> = {}): MediaItem {
    return {
        hash: "h_" + Math.random().toString(36).slice(2, 8),
        name: "item.jpg",
        created_at: "2024-01-15T10:00:00Z",
        ...over,
    };
}

function makeStore(): MediaStore {
    const mockRoot = {
        uiStore: { setError: () => {}, setSuccess: () => {}, setLoading: () => {} },
        authStore: { user: api.state.currentUser },
    } as unknown as RootStore;
    return new MediaStore(mockRoot);
}

beforeEach(() => {
    api.get.mockClear();
    api.post.mockClear();
    api.handlers.length = 0;
    Object.assign(api.state, {
        imagePool: [],
        videoPool: [],
        allPool: [],
        noExifPool: [],
        totalImage: 0,
        totalVideo: 0,
        totalAll: 0,
        totalNoExif: 0,
        searchResults: [],
        searchTotal: 0,
        mapReject: false,
        deviceIds: [],
        places: [],
        rejectMetadata: false,
    });
    api.handlers.push(
        { verb: "get", re: /^\/video_thumbnails$/, fn: (_u, u) => {
            const page = Number(u.searchParams.get("page") || 1);
            const limit = Number(u.searchParams.get("limit") || 50);
            const start = (page - 1) * limit;
            return { data: { thumbnails: api.state.videoPool.slice(start, start + limit), total: api.state.totalVideo, page, limit }, status: 200 };
        } },
        { verb: "get", re: /^\/media_thumbnails$/, fn: (_u, u) => {
            const page = Number(u.searchParams.get("page") || 1);
            const limit = Number(u.searchParams.get("limit") || 50);
            const start = (page - 1) * limit;
            return { data: { thumbnails: api.state.allPool.slice(start, start + limit), total: api.state.totalAll, page, limit }, status: 200 };
        } },
        { verb: "get", re: /^\/image_thumbnails$/, fn: (_u, u) => {
            const page = Number(u.searchParams.get("page") || 1);
            const limit = Number(u.searchParams.get("limit") || 50);
            const start = (page - 1) * limit;
            if (u.searchParams.get("no_exif") === "true") {
                return { data: { thumbnails: api.state.noExifPool.slice(start, start + limit), total: api.state.totalNoExif, page, limit }, status: 200 };
            }
            return { data: { thumbnails: api.state.imagePool.slice(start, start + limit), total: api.state.totalImage, page, limit }, status: 200 };
        } },
        { verb: "get", re: /^\/device_ids$/, fn: () => ({ data: { device_ids: api.state.deviceIds }, status: 200 }) },
        { verb: "get", re: /^\/search\/places/, fn: () => ({ data: api.state.places, status: 200 }) },
        { verb: "get", re: /^\/image\/random$/, fn: () => ({ data: { hash: "rnd", name: "rnd.jpg", created_at: "2024-01-01T00:00:00Z", place: "Prague" }, status: 200 }) },
        { verb: "get", re: /^\/search\/images/, fn: (_u, u) => {
            const offset = Number(u.searchParams.get("offset") || 0);
            const limit = Number(u.searchParams.get("limit") || 50);
            return { data: { results: api.state.searchResults.slice(offset, offset + limit), total: api.state.searchTotal }, status: 200 };
        } },
        { verb: "get", re: /^\/map\/media/, fn: () => {
            if (api.state.mapReject) throw new Error("map down");
            return { data: { points: [], total: 0 }, status: 200 };
        } },
        { verb: "get", re: /^\/image\/[^/]+\/metadata$/, fn: (_u, u) => {
            if (api.state.rejectMetadata) throw new Error("meta down");
            const hash = u.pathname.split("/")[2];
            return { data: { hash, name: "x", description: "desc", place: null, created_at: "2024-01-01T00:00:00Z", exif: null, starred: false }, status: 200 };
        } },
        { verb: "get", re: /^\/image\/[^/]+$/, fn: (_u, u) => ({ data: { hash: u.pathname.split("/")[2], name: "img", created_at: "" }, status: 200 }) },
        { verb: "post", re: /^\/image\/[^/]+\/star$/, fn: () => ({ data: { starred: true }, status: 200 }) },
        { verb: "post", re: /^\/video\/[^/]+\/star$/, fn: () => ({ data: { starred: true }, status: 200 }) },
        { verb: "post", re: /^\/(image|video)\/[^/]+\/delete$/, fn: () => ({ data: {}, status: 200 }) },
        { verb: "post", re: /^\/image\/[^/]+\/orientation$/, fn: (_u, _url, body) => {
            const b = body as { rotate?: string; orientation?: number } | undefined;
            return { data: { status: "success", orientation: b?.rotate === "cw" ? 6 : (b?.orientation ?? 1), orientation_label: "Portrait" }, status: 200 };
        } },
        { verb: "post", re: /^\/image\/[^/]+\/place$/, fn: (_u, _url, body) => {
            const b = body as { place?: string | null; latitude?: number; longitude?: number } | undefined;
            return { data: { status: "success", place: b?.place ?? null, latitude: b?.latitude, longitude: b?.longitude }, status: 200 };
        } },
    );
});

describe("MediaStore all-media fetches", () => {
    it("fetchAllMedia picks the right endpoint per type filter", async () => {
        const s = makeStore();
        s.filters.allMediaTypeFilter = "all";
        await s.fetchAllMedia(1, 50);
        expect(api.get.mock.calls.some(c => String(c[0]).startsWith("/media_thumbnails"))).toBe(true);

        s.filters.allMediaTypeFilter = "image";
        await s.fetchAllMedia(1, 50);
        expect(api.get.mock.calls.some(c => String(c[0]).startsWith("/image_thumbnails"))).toBe(true);

        s.filters.allMediaTypeFilter = "video";
        await s.fetchAllMedia(1, 50);
        expect(api.get.mock.calls.some(c => String(c[0]).startsWith("/video_thumbnails"))).toBe(true);
    });

    it("fetchAllMedia appends and tracks hasMore", async () => {
        api.state.allPool = [item({}), item({}), item({})];
        api.state.totalAll = 3;
        const s = makeStore();
        await s.fetchAllMedia(1, 2);
        expect(s.allMedia).toHaveLength(2);
        expect(s.allMediaHasMore).toBe(true);
        await s.fetchAllMedia(2, 2, true);
        expect(s.allMedia).toHaveLength(3);
        expect(s.allMediaHasMore).toBe(false);
        expect(s.isLoadingMoreAllMedia).toBe(false);
    });
});

describe("MediaStore no-EXIF + loadMore", () => {
    it("fetchNoExifImages requests the no_exif page and tracks hasMore", async () => {
        api.state.noExifPool = [item({}), item({})];
        api.state.totalNoExif = 2;
        const s = makeStore();
        await s.fetchNoExifImages(1, 1);
        const called = api.get.mock.calls.find(c => String(c[0]).includes("/image_thumbnails"))!;
        expect(new URL(String(called[0]), "http://localhost").searchParams.get("no_exif")).toBe("true");
        expect(s.noExifImages).toHaveLength(1);
        expect(s.noExifHasMore).toBe(true);

        await s.fetchNoExifImages(2, 1, true);
        expect(s.noExifImages).toHaveLength(2);
        expect(s.noExifHasMore).toBe(false);
        expect(s.isLoadingNoExif).toBe(false);
    });

    it("fetchNoExifImages failure surfaces an error", async () => {
        api.handlers.length = 0;
        api.handlers.push({ verb: "get", re: /^\/image_thumbnails$/, fn: () => { throw new Error("down"); } });
        const setError = vi.fn();
        const s = new MediaStore({ uiStore: { setError, setLoading: () => {} } } as unknown as RootStore);
        await s.fetchNoExifImages();
        expect(setError).toHaveBeenCalledWith("Failed to fetch no-EXIF images");
        expect(s.isLoadingNoExif).toBe(false);
    });

    it("loadMore* guards respect the hasMore/loading flags", async () => {
        const s = makeStore();
        s.allMediaHasMore = false;
        s.noExifHasMore = false;
        s.loadMoreAllMedia();
        s.loadMoreNoExif();
        expect(api.get).not.toHaveBeenCalled();

        s.noExifHasMore = true;
        s.isLoadingNoExif = true;
        s.loadMoreNoExif();
        expect(api.get).not.toHaveBeenCalled();

        s.noExifHasMore = true;
        s.isLoadingNoExif = false;
        s.loadMoreNoExif();
        expect(api.get.mock.calls.some(c => String(c[0]).includes("/image_thumbnails"))).toBe(true);

        api.get.mockClear();
        s.allMediaHasMore = true;
        s.searchMode = false;
        s.isSearching = false;
        s.isLoadingMoreSearch = false;
        s.isLoadingMoreAllMedia = false;
        s.loadMoreAllMedia();
        expect(api.get.mock.calls.some(c => String(c[0]).includes("/media_thumbnails"))).toBe(true);
    });

    it("loadMoreAllMedia pages via search in search mode", async () => {
        api.state.searchResults = [item({}), item({})];
        api.state.searchTotal = 2;
        const s = makeStore();
        s.searchMode = true;
        s.searchQuery = "q";
        s.allMediaHasMore = true;
        s.isSearching = false;

        await s.loadMoreAllMedia();
        await new Promise(r => setTimeout(r, 0));

        expect(s.isLoadingMoreSearch).toBe(false);
    });
});

describe("MediaStore browsing + search actions", () => {
    it("applyFilters resets paging and refetches the grid", async () => {
        const s = makeStore();
        s.allMedia = [item({})];
        s.allMediaCurrentPage = 9;

        await s.applyFilters();

        expect(s.allMediaCurrentPage).toBe(1);
        expect(s.allMedia).toEqual([]);
        expect(api.get.mock.calls.some(c => String(c[0]).includes("/media_thumbnails"))).toBe(true);
        // The removed images/videos pipelines must no longer be fetched.
        expect(api.get.mock.calls.some(c => String(c[0]).includes("/image_thumbnails"))).toBe(false);
        expect(api.get.mock.calls.some(c => String(c[0]).includes("/video_thumbnails"))).toBe(false);
    });

    it("applyFilters also refreshes map points when the map is active", async () => {
        const s = makeStore();
        s.mapActive = true;
        await s.applyFilters();
        expect(api.get.mock.calls.some(c => String(c[0]).includes("/map/media"))).toBe(true);
    });

    it("performSearch is a no-op for an empty query", async () => {
        const s = makeStore();
        api.get.mockClear();
        await s.performSearch("   ");
        expect(api.get).not.toHaveBeenCalled();
    });

    it("performSearch maps all filter params", async () => {
        api.state.searchResults = [item({ thumbnail_url: "/api/thumb/x" })];
        api.state.searchTotal = 1;
        const s = makeStore();
        s.filters.selectedDeviceId = "devA";
        s.filters.starredOnly = true;
        s.filters.startDate = "2024-01-01";
        s.filters.selectedLabelId = 5;
        s.filters.location = { name: "Prague", latitude: 50.08, longitude: 14.4, admin_level: 4, country_code: "CZ", display_name: "Prague, CZ" };
        s.filters.locationRadiusKm = 25;
        s.minSimilarity = 0.2;

        await s.performSearch("sunset");

        const called = api.get.mock.calls.find(c => String(c[0]).includes("/search/images"))!;
        const params = new URL(String(called[0]), "http://localhost").searchParams;
        expect(params.get("query")).toBe("sunset");
        expect(params.get("device_id")).toBe("devA");
        expect(params.get("starred_only")).toBe("true");
        expect(params.get("start_date")).toBe("2024-01-01");
        expect(params.get("label_id")).toBe("5");
        expect(params.get("location_lat")).toBe("50.08");
        expect(params.get("location_radius_km")).toBe("25");
        expect(params.get("min_similarity")).toBe("0.2");
        expect(params.get("mode")).toBe("semantic");
        expect(s.allMedia).toHaveLength(1);
        expect(s.allMedia[0].thumbnailUrl).toBe("/api/thumb/x");
        expect(s.totalAllMedia).toBe(1);
    });

    it("performSearch append past the total guards against stale pages", async () => {
        const s = makeStore();
        s.searchMode = true;
        s.searchOffset = 100;
        s.totalAllMedia = 50;
        await s.performSearch("q", true);
        expect(s.allMediaHasMore).toBe(false);
        expect(s.isLoadingMoreSearch).toBe(false);
    });

    it("performSearch failure surfaces an error", async () => {
        api.handlers.length = 0;
        api.handlers.push({ verb: "get", re: /^\/search\/images/, fn: () => { throw new Error("down"); } });
        const setError = vi.fn();
        const s = new MediaStore({ uiStore: { setError } } as unknown as RootStore);
        await s.performSearch("q");
        expect(setError).toHaveBeenCalledWith("Search failed");
        expect(s.isSearching).toBe(false);
    });

    it("clearAllFilters restores all defaults", () => {
        const s = makeStore();
        s.searchQuery = "x";
        s.filters.startDate = "a";
        s.filters.endDate = "b";
        s.filters.location = { name: "x", latitude: 0, longitude: 0, admin_level: 1, country_code: null, display_name: "" };
        s.locationQuery = "y";
        s.filters.starredOnly = true;
        s.filters.selectedLabelId = 3;
        s.filters.allMediaTypeFilter = "video";
        s.filters.selectedDeviceId = "d";
        s.minSimilarity = 0.5;
        s.filters.locationRadiusKm = 99;
        s.sortBy = "size";
        s.sortOrder = "asc";

        s.clearAllFilters();

        expect(s.searchQuery).toBe("");
        expect(s.filters.startDate).toBe("");
        expect(s.filters.endDate).toBe("");
        expect(s.filters.location).toBeNull();
        expect(s.locationQuery).toBe("");
        expect(s.filters.starredOnly).toBe(false);
        expect(s.filters.selectedLabelId).toBeNull();
        expect(s.filters.allMediaTypeFilter).toBe("all");
        expect(s.filters.selectedDeviceId).toBe("all");
        expect(s.minSimilarity).toBe(0.08);
        expect(s.filters.locationRadiusKm).toBe(10);
        expect(s.sortBy).toBe("date");
        expect(s.sortOrder).toBe("desc");
    });

    it("setMinSimilarity debounces a re-search in search mode", async () => {
        vi.useFakeTimers();
        try {
            api.state.searchResults = [item({})];
            api.state.searchTotal = 1;
            const s = makeStore();
            s.searchMode = true;
            s.searchQuery = "q";
            api.get.mockClear();

            s.setMinSimilarity(0.3);
            expect(api.get).not.toHaveBeenCalled();

            await vi.advanceTimersByTimeAsync(301);
            expect(api.get.mock.calls.some(c => String(c[0]).includes("/search/images"))).toBe(true);
            expect(s.minSimilarity).toBe(0.3);
        } finally {
            vi.useRealTimers();
        }
    });
});

describe("MediaStore lightbox", () => {
    it("openMediaLightbox loads full media and image metadata", async () => {
        const s = makeStore();
        const target = item({ media_type: "image" });
        s.allMedia = [target];

        await s.openMediaLightbox(0, "all");

        expect(s.lightboxSource).toBe("all");
        expect(s.selectedMediaIndex).toBe(0);
        expect(s.fullMediaUrl).toBe(`/api/image/${target.hash}`);
        expect(s.imageMetadata?.hash).toBe(target.hash);
        expect(s.lastLoadedMetadataHash).toBe(target.hash);
    });

    it("openMediaLightbox clears metadata for videos", async () => {
        const s = makeStore();
        const target = item({ media_type: "video" });
        s.allMedia = [target];

        await s.openMediaLightbox(0, "all");

        expect(s.fullMediaUrl).toBe(`/api/video/${target.hash}`);
        expect(s.imageMetadata).toBeNull();
    });

    it("nextMedia / previousMedia navigate and include comparison in compare mode", async () => {
        const s = makeStore();
        const a = item({ media_type: "image" });
        const b = item({ media_type: "image" });
        s.allMedia = [a, b];
        s.lightboxSource = "all";
        s.selectedMediaIndex = 0;
        s.compareMode = true;

        await s.nextMedia();
        expect(s.selectedMediaIndex).toBe(1);
        expect(s.compareMode).toBe(true);
        // comparison loads the item after the *next* one: not available here

        await s.previousMedia();
        expect(s.selectedMediaIndex).toBe(0);
    });

    it("nextMedia/previousMedia do nothing at the ends", async () => {
        const s = makeStore();
        s.allMedia = [item({}), item({})];
        s.lightboxSource = "all";
        s.selectedMediaIndex = 1;
        await s.nextMedia();
        expect(s.selectedMediaIndex).toBe(1);

        s.selectedMediaIndex = 0;
        await s.previousMedia();
        expect(s.selectedMediaIndex).toBe(0);
    });

    it("toggleCompareMode enables comparison and loads the next item", async () => {
        const s = makeStore();
        const a = item({ media_type: "image" });
        const b = item({ media_type: "image" });
        s.allMedia = [a, b];
        s.lightboxSource = "all";
        s.selectedMediaIndex = 0;

        await s.openMediaLightbox(0, "all");
        await s.toggleCompareMode();

        expect(s.compareMode).toBe(true);
        expect(s.comparisonMediaUrl).toBe(`/api/image/${b.hash}`);

        await s.toggleCompareMode();
        expect(s.compareMode).toBe(false);
        expect(s.comparisonMediaUrl).toBeNull();
    });

    it("closeMediaLightbox resets all lightbox state", async () => {
        const s = makeStore();
        s.selectedMediaIndex = 2;
        s.fullMediaUrl = "/api/image/x";
        s.comparisonMediaUrl = "/api/image/y";
        s.compareMode = true;
        s.imageMetadata = { hash: "x", name: "n", description: null, place: null, created_at: "", exif: null, starred: false };
        s.lastLoadedMetadataHash = "x";

        await s.closeMediaLightbox();

        expect(s.selectedMediaIndex).toBeNull();
        expect(s.fullMediaUrl).toBeNull();
        expect(s.comparisonMediaUrl).toBeNull();
        expect(s.compareMode).toBe(false);
        expect(s.imageMetadata).toBeNull();
        expect(s.lastLoadedMetadataHash).toBeNull();
    });

    it("fetchRandomImage returns a plain cookie-authenticated URL on success", async () => {
        const s = makeStore();
        const result = await s.fetchRandomImage();
        expect(result?.hash).toBe("rnd");
        expect(result?.thumbnailUrl).toBe("/api/image/rnd");
    });

    it("fetchRandomImage returns null on failure", async () => {
        api.handlers.length = 0;
        api.handlers.push({ verb: "get", re: /^\/image\/random$/, fn: () => { throw new Error("down"); } });
        const s = makeStore();
        expect(await s.fetchRandomImage(true, [1, 2])).toBeNull();
    });
});

describe("MediaStore devices + location", () => {
    it("fetchDeviceIds loads ids and auto-selects a single device for non-admins", async () => {
        api.state.deviceIds = ["devA"];
        api.state.currentUser = { role: "user" };
        const s = makeStore();
        s.filters.selectedDeviceId = "all";

        await s.fetchDeviceIds();

        expect(s.deviceIds).toEqual(["devA"]);
        expect(s.filters.selectedDeviceId).toBe("devA");
    });

    it("fetchDeviceIds does not auto-select for admins", async () => {
        api.state.deviceIds = ["devA"];
        api.state.currentUser = { role: "admin" };
        const s = makeStore();
        s.filters.selectedDeviceId = "all";

        await s.fetchDeviceIds();

        expect(s.deviceIds).toEqual(["devA"]);
        expect(s.filters.selectedDeviceId).toBe("all");
    });

    it("setLocationQuery fetches suggestions for 3+ chars and clears below", async () => {
        const s = makeStore();
        s.setLocationQuery("ab");
        expect(s.locationSuggestions).toEqual([]);
        expect(s.locationQuery).toBe("ab");

        api.state.places = [{ name: "Prague" }];
        await new Promise<void>(resolve => {
            api.get.mockClear();
            s.setLocationQuery("pra");
            // Suggestions are debounced by 250ms in the store.
            setTimeout(resolve, 300);
        });
        expect(s.locationSuggestions).toEqual([{ name: "Prague" }]);
    });

    it("fetchLocationSuggestions failure surfaces an error and clears suggestions", async () => {
        api.handlers.length = 0;
        api.handlers.push({ verb: "get", re: /^\/search\/places/, fn: () => { throw new Error("down"); } });
        const setError = vi.fn();
        const s = new MediaStore({ uiStore: { setError } } as unknown as RootStore);

        s.setLocationQuery("pra");
        await new Promise(r => setTimeout(r, 300)); // outlasts the 250ms debounce

        expect(setError).toHaveBeenCalledWith("Failed to fetch location suggestions");
        expect(s.locationSuggestions).toEqual([]);
        expect(s.isLoadingLocationSuggestions).toBe(false);
    });

    it("selectLocation / setLocationRadiusKm / clearLocationFilter", () => {
        const s = makeStore();
        const loc = { name: "Prague", latitude: 50, longitude: 14, admin_level: 4, country_code: "CZ", display_name: "Prague" };
        s.selectLocation(loc);
        expect(s.filters.location).toEqual(loc);
        expect(s.locationSuggestions).toEqual([]);

        s.setLocationRadiusKm(50);
        expect(s.filters.locationRadiusKm).toBe(50);

        s.clearLocationFilter();
        expect(s.filters.location).toBeNull();
        expect(s.locationQuery).toBe("");
    });
});

describe("MediaStore map error + grouping", () => {
    it("fetchMapPoints failure sets the map error", async () => {
        api.state.mapReject = true;
        const s = makeStore();
        await s.fetchMapPoints();
        expect(s.mapError).toBe("Failed to load map");
        expect(s.isMapLoading).toBe(false);
    });

    it("groupMedia sorts by quality ascending with place grouping", () => {
        const s = makeStore();
        s.allMedia = [
            item({ place: "A", aesthetic_score: 0.1 }),
            item({ place: "B", aesthetic_score: 0.9 }),
            item({ place: "A", aesthetic_score: 0.5 }),
        ];
        s.allMediaGroupBy = "place";
        s.sortBy = "quality";
        s.sortOrder = "asc";

        const groups = s.groupedAllMedia;
        const groupA = groups.find(g => g.date === "A");
        expect(groupA?.items.map(i => i.aesthetic_score)).toEqual([0.1, 0.5]);
    });

    it("formatDisplayDate renders Today for the current date", () => {
        const today = new Date().toISOString().split("T")[0];
        const s = makeStore();
        s.allMedia = [item({ created_at: `${today}T10:00:00Z` })];
        s.allMediaGroupBy = "day";
        expect(s.groupedAllMedia[0].displayDate).toBe("Today");
    });
});

describe("MediaStore updateImageOrientation and updateImagePlace", () => {
    it("updateImageOrientation calls orientation endpoint, updates metadata and busts URLs", async () => {
        const s = makeStore();
        s.imageMetadata = {
            hash: "h1",
            name: "test.jpg",
            description: null,
            place: null,
            created_at: "2024-01-01",
            exif: null,
            starred: false,
            orientation: 1,
            orientation_label: "Landscape",
        };
        s.allMedia = [item({ hash: "h1", thumbnailUrl: "/api/thumbnail/h1" })];

        const res = await s.updateImageOrientation("h1", "cw");
        expect(res.orientation).toBe(6);
        expect(s.imageMetadata.orientation).toBe(6);
        expect(s.imageMetadata.orientation_label).toBe("Portrait");
        expect(s.allMedia[0].thumbnailUrl).toContain("/api/thumbnail/h1");
        expect(s.allMedia[0].thumbnailUrl).toContain("v=");
        expect(s.fullMediaUrl).toContain("/api/image/h1");
        expect(s.fullMediaUrl).toContain("v=");
    });

    it("updateImagePlace calls place endpoint and updates metadata and media list", async () => {
        const s = makeStore();
        s.imageMetadata = {
            hash: "h1",
            name: "test.jpg",
            description: null,
            place: null,
            created_at: "2024-01-01",
            exif: null,
            starred: false,
        };
        s.allMedia = [item({ hash: "h1", place: "Old Place" })];

        const res = await s.updateImagePlace("h1", "Paris, France", 48.85, 2.35);
        expect(res.place).toBe("Paris, France");
        expect(s.imageMetadata.place).toBe("Paris, France");
        expect(s.imageMetadata.latitude).toBe(48.85);
        expect(s.imageMetadata.longitude).toBe(2.35);
        expect(s.allMedia[0].place).toBe("Paris, France");

        // Clear place
        await s.updateImagePlace("h1", null);
        expect(s.imageMetadata.place).toBeNull();
        expect(s.allMedia[0].place).toBeUndefined();
    });
});
