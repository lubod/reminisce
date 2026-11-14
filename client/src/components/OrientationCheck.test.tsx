import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { StoreContext } from "../stores/RootStore";
import { OrientationCheck } from "./OrientationCheck";

function makeMediaStore(over: Record<string, unknown> = {}) {
    return {
        noExifImages: [],
        totalNoExif: 0,
        noExifHasMore: false,
        isLoadingNoExif: false,
        noExifWrongOrientationOnly: true,
        fetchNoExifImages: vi.fn(),
        loadMoreNoExif: vi.fn(),
        setNoExifWrongOrientationOnly: vi.fn(),
        deleteMedia: vi.fn(),
        updateImageOrientation: vi.fn(),
        ...over,
    };
}

function renderOrientation(
    mediaStore: ReturnType<typeof makeMediaStore>,
    role: string = "user"
) {
    return render(
        <StoreContext.Provider value={{ mediaStore, authStore: { user: { role } } } as never}>
            <OrientationCheck />
        </StoreContext.Provider>,
    );
}

describe("OrientationCheck", () => {
    beforeEach(() => {
        vi.clearAllMocks();
        class MockIntersectionObserver {
            observe = vi.fn();
            disconnect = vi.fn();
            unobserve = vi.fn();
        }
        vi.stubGlobal("IntersectionObserver", MockIntersectionObserver);
    });

    afterEach(() => {
        vi.unstubAllGlobals();
    });

    it("fetches EXIF-less images on mount and defaults wrong-orientation toggle to pressed", () => {
        const mediaStore = makeMediaStore();
        renderOrientation(mediaStore);
        expect(mediaStore.fetchNoExifImages).toHaveBeenCalledWith(1, 50, false);

        const toggle = screen.getByRole("button", { name: "Wrong orientation only" });
        expect(toggle.getAttribute("aria-pressed")).toBe("true");
        expect(screen.getByText(/photos with no EXIF metadata/)).toBeTruthy();
    });

    it("toggles wrong-orientation filter when clicked", async () => {
        const mediaStore = makeMediaStore({ noExifWrongOrientationOnly: true });
        renderOrientation(mediaStore);

        const user = userEvent.setup();
        await user.click(screen.getByRole("button", { name: "Wrong orientation only" }));
        expect(mediaStore.setNoExifWrongOrientationOnly).toHaveBeenCalledWith(false);
    });

    it("renders detected orientation badges and allows switching to all no-EXIF photos from empty state", async () => {
        const populated = makeMediaStore({
            noExifWrongOrientationOnly: true,
            totalNoExif: 2,
            noExifImages: [
                { hash: "h1", name: "a.jpg", created_at: "2024-01-01", thumbnailUrl: "/api/thumbnail/h1", orientation: 6 },
                { hash: "h2", name: "b.jpg", created_at: "2024-01-02", thumbnailUrl: "/api/thumbnail/h2", orientation: 3 },
            ],
        });
        const { unmount } = renderOrientation(populated);
        expect(screen.getByText("90° CW")).toBeTruthy();
        expect(screen.getByText("180°")).toBeTruthy();
        unmount();

        const emptyFiltered = makeMediaStore({
            noExifWrongOrientationOnly: true,
            totalNoExif: 0,
            noExifImages: [],
        });
        renderOrientation(emptyFiltered);
        expect(screen.getByText(/detected with wrong orientation/i)).toBeTruthy();

        const user = userEvent.setup();
        await user.click(screen.getByRole("button", { name: "Show all photos without EXIF" }));
        expect(emptyFiltered.setNoExifWrongOrientationOnly).toHaveBeenCalledWith(false);
    });
});
