import { describe, it, expect, vi, beforeEach } from "vitest";
import { PersonStore, type Person, type PersonImage } from "./PersonStore";
import type { RootStore } from "./RootStore";

const mocks = vi.hoisted(() => {
    const routes: Record<string, () => unknown> = {};
    const get = vi.fn(async (url: string) => {
        const fn = routes["GET " + url];
        if (!fn) return { data: {}, status: 200 };
        return fn();
    });
    const put = vi.fn(async (url: string) => {
        const fn = routes["PUT " + url];
        if (!fn) return { data: {}, status: 200 };
        return fn();
    });
    const post = vi.fn(async (url: string) => {
        const fn = routes["POST " + url];
        if (!fn) return { data: {}, status: 200 };
        return fn();
    });
    const clear = () => { for (const k of Object.keys(routes)) delete routes[k]; };
    return { api: { get, post, put }, routes, clear };
});

vi.mock("../api/axiosConfig", () => ({ __esModule: true, default: mocks.api }));

function person(over: Partial<Person> = {}): Person {
    return {
        id: 1,
        name: "Alice",
        face_count: 3,
        representative_face_hash: "h1",
        representative_face_deviceid: "dev1",
        representative_face_id: 10,
        representative_bbox: [0, 0, 10, 10],
        representative_face_url: "/api/face/10/thumbnail",
        created_at: "2024-01-01T00:00:00Z",
        updated_at: "2024-01-01T00:00:00Z",
        ...over,
    };
}

function personImage(over: Partial<PersonImage> = {}): PersonImage {
    return {
        hash: "img1",
        deviceid: "dev1",
        name: "img1.jpg",
        created_at: "2024-01-01T00:00:00Z",
        bbox: [0, 0, 5, 5],
        confidence: 0.98,
        thumbnail_url: "/api/image/img1/thumb",
        face_id: 10,
        ...over,
    };
}

function makeStore(): { store: PersonStore; errors: string[] } {
    const errors: string[] = [];
    const mockRoot = { uiStore: { setError: (m: string) => errors.push(m) } } as unknown as RootStore;
    return { store: new PersonStore(mockRoot), errors };
}

describe("PersonStore", () => {
    beforeEach(() => {
        mocks.clear();
        mocks.api.get.mockClear();
        mocks.api.post.mockClear();
        mocks.api.put.mockClear();
    });

    it("fetchPersons populates the list, sets total and pages forward", async () => {
        mocks.routes["GET /persons?page=1&limit=50"] = () => ({ data: { persons: [person({ id: 1 }), person({ id: 2 })], total: 5 } });
        const { store } = makeStore();
        await store.fetchPersons();

        expect(store.persons).toHaveLength(2);
        expect(store.persons[0].thumbnailUrl).toBe("/api/face/10/thumbnail");
        expect(store.total).toBe(5);
        expect(store.hasMore).toBe(true);
        expect(store.page).toBe(2);
        expect(store.isLoading).toBe(false);
    });

    it("fetchPersons reset replaces the list and resets to page 1", async () => {
        mocks.routes["GET /persons?page=1&limit=50"] = () => ({ data: { persons: [person({ id: 3 })], total: 1 } });
        const { store } = makeStore();
        store.persons = [person({ id: 99 })];
        store.page = 9;
        store.hasMore = false;

        await store.fetchPersons(true);
        expect(store.persons).toHaveLength(1);
        expect(store.persons[0].id).toBe(3);
        expect(store.page).toBe(1);
        expect(store.hasMore).toBe(false);
    });

    it("fetchPersons is a no-op when already loading or exhausted", async () => {
        const { store } = makeStore();
        store.isLoading = true;
        await store.fetchPersons();
        expect(mocks.api.get).not.toHaveBeenCalled();

        store.isLoading = false;
        store.hasMore = false;
        await store.fetchPersons();
        expect(mocks.api.get).not.toHaveBeenCalled();
    });

    it("fetchPersons failure surfaces an error and resets loading", async () => {
        mocks.routes["GET /persons?page=1&limit=50"] = () => { throw new Error("x"); };
        const { store, errors } = makeStore();
        await store.fetchPersons();
        expect(errors).toContain("Failed to fetch persons");
        expect(store.isLoading).toBe(false);
    });

    it("fetchPerson loads a single person and then their images", async () => {
        mocks.routes["GET /persons/7"] = () => ({ data: { person: person({ id: 7, representative_face_url: null }) } });
        mocks.routes["GET /persons/7/images?limit=60&offset=0"] = () => ({ data: { images: [personImage()], total: 1 } });
        const { store } = makeStore();

        await store.fetchPerson(7);

        expect(store.selectedPerson?.id).toBe(7);
        expect(store.selectedPerson?.thumbnailUrl).toBeUndefined(); // no face url
        expect(store.personImages).toHaveLength(1);
        expect(store.imagesTotal).toBe(1);
        expect(store.isLoading).toBe(false);
    });

    it("fetchPerson failure surfaces an error", async () => {
        mocks.routes["GET /persons/7"] = () => { throw new Error("x"); };
        const { store, errors } = makeStore();
        await store.fetchPerson(7);
        expect(errors).toContain("Failed to fetch person");
        expect(store.isLoading).toBe(false);
    });

    it("selectPerson selects and loads images", async () => {
        mocks.routes["GET /persons/3/images?limit=60&offset=0"] = () => ({ data: { images: [personImage()], total: 1 } });
        const { store } = makeStore();
        await store.selectPerson(person({ id: 3 }));
        expect(store.selectedPerson?.id).toBe(3);
        expect(store.personImages).toHaveLength(1);
    });

    it("fetchPersonImages reset false appends more images and tracks offsets", async () => {
        mocks.routes["GET /persons/3/images?limit=60&offset=60"] = () => ({ data: { images: [personImage({ hash: "img2" })], total: 120 } });
        const { store } = makeStore();
        store.selectedPerson = person({ id: 3 });
        store.imagesOffset = 60;
        store.personImages = [personImage()];

        await store.fetchPersonImages(3, false);
        expect(store.personImages).toHaveLength(2);
        expect(store.imagesOffset).toBe(61);
        expect(store.imagesHasMore).toBe(true);
        expect(store.isLoadingMoreImages).toBe(false);
    });

    it("fetchPersonImages failure surfaces an error", async () => {
        mocks.routes["GET /persons/3/images?limit=60&offset=0"] = () => { throw new Error("x"); };
        const { store, errors } = makeStore();
        await store.fetchPersonImages(3);
        expect(errors).toContain("Failed to fetch person images");
        expect(store.isLoadingImages).toBe(false);
        expect(store.isLoadingMoreImages).toBe(false);
    });

    it("loadMorePersonImages guards on missing selection / no more / busy", async () => {
        mocks.routes["GET /persons/3/images?limit=60&offset=0"] = () => ({ data: { images: [], total: 0 } });
        const { store } = makeStore();

        await store.loadMorePersonImages();
        expect(mocks.api.get).not.toHaveBeenCalled();

        store.selectedPerson = person({ id: 3 });
        store.imagesHasMore = true;
        store.isLoadingMoreImages = true;
        await store.loadMorePersonImages();
        expect(mocks.api.get).not.toHaveBeenCalled();

        store.isLoadingMoreImages = false;
        store.imagesHasMore = false;
        await store.loadMorePersonImages();
        expect(mocks.api.get).not.toHaveBeenCalled();
    });

    it("loadMorePersonImages fetches the next page when eligible", async () => {
        mocks.routes["GET /persons/3/images?limit=60&offset=60"] = () => ({ data: { images: [personImage()], total: 2 } });
        const { store } = makeStore();
        store.selectedPerson = person({ id: 3 });
        store.imagesHasMore = true;
        store.imagesOffset = 60;
        await store.loadMorePersonImages();
        expect(store.personImages).toHaveLength(1);
        expect(store.isLoadingMoreImages).toBe(false);
    });

    it("updatePersonName updates the list and selection when present", async () => {
        const p1 = person({ id: 1 });
        const { store } = makeStore();
        store.persons = [p1, person({ id: 2 })];
        store.selectedPerson = p1;

        await store.updatePersonName(1, "Bob");
        expect(store.persons[0].name).toBe("Bob");
        expect(store.selectedPerson.name).toBe("Bob");
    });

    it("updatePersonName ignores unknown ids and reports failures", async () => {
        const { store, errors } = makeStore();
        await store.updatePersonName(99, "X");
        expect(store.persons).toHaveLength(0);

        mocks.routes["PUT /persons/99/name"] = () => { throw new Error("x"); };
        await store.updatePersonName(99, "Y");
        expect(errors).toContain("Failed to update person name");
    });

    it("setRepresentativeFace refreshes the person on success", async () => {
        mocks.routes["PUT /persons/1/representative_face"] = () => ({ data: {} });
        mocks.routes["GET /persons/1"] = () => ({ data: { person: person({ id: 1 }) } });
        mocks.routes["GET /persons/1/images?limit=60&offset=0"] = () => ({ data: { images: [], total: 0 } });
        const { store } = makeStore();

        await store.setRepresentativeFace(1, 10);
        expect(store.selectedPerson?.id).toBe(1);
    });

    it("setRepresentativeFace reports failures", async () => {
        mocks.routes["PUT /persons/1/representative_face"] = () => { throw new Error("x"); };
        const { store, errors } = makeStore();
        await store.setRepresentativeFace(1, 10);
        expect(errors).toContain("Failed to set representative face");
    });

    it("mergePersons posts, then refreshes the list and target", async () => {
        mocks.routes["POST /persons/merge"] = () => ({ data: {} });
        mocks.routes["GET /persons?page=1&limit=50"] = () => ({ data: { persons: [], total: 0 } });
        mocks.routes["GET /persons/5"] = () => ({ data: { person: person({ id: 5 }) } });
        mocks.routes["GET /persons/5/images?limit=60&offset=0"] = () => ({ data: { images: [], total: 0 } });
        const { store } = makeStore();

        await store.mergePersons([1, 2], 5);
        expect(mocks.api.post).toHaveBeenCalledWith("/persons/merge", {
            source_person_ids: [1, 2],
            target_person_id: 5,
        });
        expect(store.selectedPerson?.id).toBe(5);
    });

    it("mergePersons reports failures", async () => {
        mocks.routes["POST /persons/merge"] = () => { throw new Error("x"); };
        const { store, errors } = makeStore();
        await store.mergePersons([1], 2);
        expect(errors).toContain("Failed to merge persons");
    });

    it("clearSelection revokes blob URLs and resets image state", () => {
        const revoke = vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => {});
        const { store } = makeStore();
        const withBlob = personImage({ thumbnailUrl: "blob:local" });
        const withoutBlob = personImage({ hash: "nonblob", thumbnailUrl: "/api/x" });
        store.selectedPerson = person();
        store.personImages = [withBlob, withoutBlob];
        store.imagesOffset = 60;
        store.imagesTotal = 5;
        store.imagesHasMore = true;

        store.clearSelection();

        expect(revoke).toHaveBeenCalledWith("blob:local");
        expect(store.selectedPerson).toBeNull();
        expect(store.personImages).toEqual([]);
        expect(store.imagesOffset).toBe(0);
        expect(store.imagesTotal).toBe(0);
        expect(store.imagesHasMore).toBe(false);
        revoke.mockRestore();
    });

    it("getAuthenticatedUrl passes URLs through unmodified", () => {
        const { store } = makeStore();
        expect(store.getAuthenticatedUrl("/api/face/1/thumbnail")).toBe("/api/face/1/thumbnail");
    });

    it("cleanup is a no-op", () => {
        const { store } = makeStore();
        expect(() => store.cleanup()).not.toThrow();
    });
});
