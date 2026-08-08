import { describe, it, expect, vi, beforeEach } from "vitest";
import { LabelStore } from "./LabelStore";
import type { RootStore } from "./RootStore";

const mocks = vi.hoisted(() => {
    const routes: Record<string, () => { data: unknown }> = {};
    const get = vi.fn(async (url: string) => {
        const fn = routes[url];
        if (!fn) return { data: { labels: [] }, status: 200 };
        return fn();
    });
    const post = vi.fn(async (url: string, body: unknown) => { void url; void body; return { data: {}, status: 200 }; });
    const del = vi.fn(async () => ({ data: {}, status: 200 }));
    const clear = () => { for (const k of Object.keys(routes)) delete routes[k]; };
    return { api: { get, post, delete: del }, routes, clear };
});

vi.mock("../api/axiosConfig", () => ({ __esModule: true, default: mocks.api }));

function makeStore(): { store: LabelStore; errors: string[]; successes: string[] } {
    const errors: string[] = [];
    const successes: string[] = [];
    const setError = (m: string) => errors.push(m);
    const setSuccess = (m: string) => successes.push(m);
    const mockRoot = { uiStore: { setError, setSuccess } } as unknown as RootStore;
    return { store: new LabelStore(mockRoot), errors, successes };
}

describe("LabelStore", () => {
    beforeEach(() => mocks.clear());

    it("fetchLabels populates labels", async () => {
        mocks.routes["/labels"] = () => ({ data: { labels: [{ id: 1, name: "Trip", color: "#3B82F6" }] } });
        const { store } = makeStore();
        await store.fetchLabels();
        expect(store.labels).toHaveLength(1);
        expect(store.labels[0].name).toBe("Trip");
        expect(store.isLoading).toBe(false);
    });

    it("fetchLabels failure surfaces an error through uiStore", async () => {
        mocks.routes["/labels"] = () => { throw new Error("x"); };
        const { store, errors } = makeStore();
        await store.fetchLabels();
        expect(errors).toContain("Failed to fetch labels");
        expect(store.isLoading).toBe(false);
    });

    it("createLabel appends the returned label with the default color", async () => {
        mocks.api.post.mockImplementationOnce(async (url: string, body: unknown) => {
            void url;
            const color = (body as { color?: string } | undefined)?.color ?? "#3B82F6";
            return { data: { id: 2, name: "New", color }, status: 200 };
        });
        const { store } = makeStore();
        const created = await store.createLabel("New");
        expect(created.id).toBe(2);
        expect(store.labels.map(l => l.id)).toContain(2);
        expect(mocks.api.post).toHaveBeenCalledWith("/labels", { name: "New", color: "#3B82F6" });
    });

    it("createLabel failure throws and surfaces an error", async () => {
        mocks.api.post.mockImplementationOnce(async () => { throw new Error("x"); });
        const { store, errors } = makeStore();
        await expect(store.createLabel("Bad")).rejects.toThrow();
        expect(errors).toContain("Failed to create label");
    });

    it("deleteLabel removes the label from state", async () => {
        mocks.routes["/labels"] = () => ({ data: { labels: [{ id: 1, name: "A" }, { id: 2, name: "B" }] } });
        const { store } = makeStore();
        await store.fetchLabels();
        await store.deleteLabel(1);
        expect(store.labels.map(l => l.id)).toEqual([2]);
        expect(mocks.api.delete).toHaveBeenCalledWith("/labels/1");
    });

    it("deleteLabel failure rethrows and surfaces an error", async () => {
        mocks.api.delete.mockImplementationOnce(async () => { throw new Error("x"); });
        const { store, errors } = makeStore();
        await expect(store.deleteLabel(1)).rejects.toThrow();
        expect(errors).toContain("Failed to delete label");
    });

    it("getImageLabels returns labels on success and [] on failure", async () => {
        mocks.routes["/images/hash1/labels"] = () => ({ data: { labels: [{ id: 3, name: "C" }] } });
        const { store } = makeStore();
        expect(await store.getImageLabels("hash1")).toHaveLength(1);

        mocks.routes["/images/hash1/labels"] = () => { throw new Error("x"); };
        expect(await store.getImageLabels("hash1")).toEqual([]);
    });

    it("addImageLabel posts and reports failures", async () => {
        mocks.api.post.mockImplementationOnce(async () => ({ data: {}, status: 200 }));
        const { store } = makeStore();
        await store.addImageLabel("h1", 3);
        expect(mocks.api.post).toHaveBeenCalledWith("/images/h1/labels", { label_id: 3 });

        mocks.api.post.mockImplementationOnce(async () => { throw new Error("x"); });
        const { store: s2, errors } = makeStore();
        await expect(s2.addImageLabel("h1", 3)).rejects.toThrow();
        expect(errors).toContain("Failed to add label");
    });

    it("removeImageLabel deletes and reports failures", async () => {
        mocks.api.delete.mockImplementationOnce(async () => ({ data: {}, status: 200 }));
        const { store } = makeStore();
        await store.removeImageLabel("h1", 3);
        expect(mocks.api.delete).toHaveBeenCalledWith("/images/h1/labels/3");

        mocks.api.delete.mockImplementationOnce(async () => { throw new Error("x"); });
        const { store: s2, errors } = makeStore();
        await expect(s2.removeImageLabel("h1", 3)).rejects.toThrow();
        expect(errors).toContain("Failed to remove label");
    });

    it("getVideoLabels returns labels on success and [] on failure", async () => {
        mocks.routes["/videos/v1/labels"] = () => ({ data: { labels: [{ id: 4, name: "D" }] } });
        const { store } = makeStore();
        expect(await store.getVideoLabels("v1")).toHaveLength(1);

        mocks.routes["/videos/v1/labels"] = () => { throw new Error("x"); };
        expect(await store.getVideoLabels("v1")).toEqual([]);
    });

    it("addVideoLabel posts and reports failures", async () => {
        mocks.api.post.mockImplementationOnce(async () => ({ data: {}, status: 200 }));
        const { store } = makeStore();
        await store.addVideoLabel("v1", 4);
        expect(mocks.api.post).toHaveBeenCalledWith("/videos/v1/labels", { label_id: 4 });

        mocks.api.post.mockImplementationOnce(async () => { throw new Error("x"); });
        const { store: s2, errors } = makeStore();
        await expect(s2.addVideoLabel("v1", 4)).rejects.toThrow();
        expect(errors).toContain("Failed to add label");
    });

    it("removeVideoLabel deletes and reports failures", async () => {
        mocks.api.delete.mockImplementationOnce(async () => ({ data: {}, status: 200 }));
        const { store } = makeStore();
        await store.removeVideoLabel("v1", 4);
        expect(mocks.api.delete).toHaveBeenCalledWith("/videos/v1/labels/4");

        mocks.api.delete.mockImplementationOnce(async () => { throw new Error("x"); });
        const { store: s2, errors } = makeStore();
        await expect(s2.removeVideoLabel("v1", 4)).rejects.toThrow();
        expect(errors).toContain("Failed to remove label");
    });
});
