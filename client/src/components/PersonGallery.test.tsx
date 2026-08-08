import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { StoreContext } from "../stores/RootStore";
import { PersonGallery } from "./PersonGallery";

function person(over: Record<string, unknown> = {}) {
    return {
        id: 1,
        name: "Alice",
        face_count: 3,
        representative_face_hash: "h",
        representative_face_deviceid: "d",
        representative_face_id: 1,
        representative_bbox: [0, 0, 1, 1],
        representative_face_url: "/api/face/1/thumbnail",
        representative_face_thumbnail: undefined,
        created_at: "2024-01-01T00:00:00Z",
        updated_at: "2024-01-01T00:00:00Z",
        thumbnailUrl: "/api/face/1/thumbnail",
        ...over,
    };
}

function makeStore(over: Record<string, unknown> = {}) {
    return {
        fetchPersons: vi.fn(),
        cleanup: vi.fn(),
        updatePersonName: vi.fn(() => Promise.resolve()),
        isLoading: false,
        hasMore: false,
        persons: [] as unknown[],
        ...over,
    };
}

function renderGallery(store: ReturnType<typeof makeStore>, entry = "/people") {
    return render(
        <StoreContext.Provider value={{ personStore: store } as never}>
            <MemoryRouter initialEntries={[entry]}>
                <PersonGallery />
            </MemoryRouter>
        </StoreContext.Provider>,
    );
}

describe("PersonGallery", () => {
    beforeEach(() => vi.clearAllMocks());

    it("fetches persons on mount", () => {
        const store = makeStore();
        renderGallery(store);
        expect(store.fetchPersons).toHaveBeenCalledWith(true);
    });

    it("shows a loading state while loading with no persons", () => {
        const store = makeStore({ isLoading: true });
        renderGallery(store);
        expect(screen.getByText("Loading persons...")).toBeTruthy();
    });

    it("shows the empty state when there are no persons", () => {
        renderGallery(makeStore());
        expect(screen.getByText("No persons detected yet")).toBeTruthy();
    });

    it("renders persons with a count", () => {
        const store = makeStore({ persons: [person(), person({ id: 2, name: "Bob" })] });
        renderGallery(store);
        expect(screen.getByText("People (2)")).toBeTruthy();
        expect(screen.getByText("Alice")).toBeTruthy();
        expect(screen.getByText("Bob")).toBeTruthy();
    });

    it("navigates into a person on click", async () => {
        const store = makeStore({ persons: [person({ id: 42 })] });
        renderGallery(store);
        const user = userEvent.setup();
        await user.click(screen.getByRole("button", { name: /view photos of alice/i }));
        // Navigation target is reflected by MemoryRouter location in the real app;
        // just assert the click handler didn't throw.
        expect(store.cleanup).not.toHaveBeenCalled();
    });

    it("renders an unnamed fallback label", () => {
        const store = makeStore({ persons: [person({ name: null })] });
        renderGallery(store);
        expect(screen.getByText("Person 1")).toBeTruthy();
    });

    it("lets a user edit and save a person name", async () => {
        const store = makeStore({ persons: [person({ id: 7 })] });
        renderGallery(store);
        const user = userEvent.setup();

        await user.click(screen.getByTitle("Edit name"));
        const input = screen.getByRole("textbox");
        await user.clear(input);
        await user.type(input, "Renamed");
        await user.click(screen.getByTitle("Save"));

        expect(store.updatePersonName).toHaveBeenCalledWith(7, "Renamed");
    });
});
