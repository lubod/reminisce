import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { StoreContext } from "../stores/RootStore";
import { TrashBrowser } from "./TrashBrowser";

function makeStore(over: Record<string, unknown> = {}) {
    return {
        fetchTrash: vi.fn(),
        isLoading: false,
        error: null as string | null,
        items: [],
        getThumbnailUrl: vi.fn((i: { hash: string }) => `/api/thumb/${i.hash}`),
        restoreItem: vi.fn(),
        ...over,
    };
}

function renderTrash(trashStore: ReturnType<typeof makeStore>) {
    return render(
        <StoreContext.Provider value={{ trashStore } as never}>
            <TrashBrowser />
        </StoreContext.Provider>,
    );
}

describe("TrashBrowser", () => {
    beforeEach(() => vi.clearAllMocks());

    it("fetches trash on mount", () => {
        const store = makeStore();
        renderTrash(store);
        expect(store.fetchTrash).toHaveBeenCalledTimes(1);
    });

    it("shows a loading indicator and no content while loading", () => {
        const store = makeStore({ isLoading: true, items: [{ hash: "h", media_type: "image", name: "x", deleted_at: "" }] });
        renderTrash(store);
        expect(screen.queryByText("Trash is empty")).toBeNull();
        expect(screen.queryByText("restore", { exact: false })).toBeNull();
    });

    it("shows the error message when present", () => {
        const store = makeStore({ error: "Deletion failed" });
        renderTrash(store);
        expect(screen.getByText("Deletion failed")).toBeTruthy();
    });

    it("shows the empty state when there are no items", () => {
        renderTrash(makeStore());
        expect(screen.getByText("Trash is empty")).toBeTruthy();
    });

    it("renders deleted items with a count and restore button", async () => {
        const store = makeStore({
            items: [
                { hash: "h1", media_type: "image", name: "a.jpg", deleted_at: "2024-01-15T00:00:00Z" },
                { hash: "h2", media_type: "video", name: "b.mp4", deleted_at: "2024-01-16T00:00:00Z" },
            ],
        });
        renderTrash(store);

        expect(screen.getByText("2 deleted items")).toBeTruthy();
        expect(screen.getByText("a.jpg")).toBeTruthy();
        expect(screen.getByText("b.mp4")).toBeTruthy();
        expect(store.getThumbnailUrl).toHaveBeenCalledTimes(2);

        const user = userEvent.setup();
        await user.click(screen.getAllByText("Restore")[0]);
        expect(store.restoreItem).toHaveBeenCalledWith("h1", "image");
    });

    it("uses singular wording for a single item", () => {
        const store = makeStore({
            items: [{ hash: "h1", media_type: "image", name: "a.jpg", deleted_at: "2024-01-15T00:00:00Z" }],
        });
        renderTrash(store);
        expect(screen.getByText("1 deleted item")).toBeTruthy();
    });
});
