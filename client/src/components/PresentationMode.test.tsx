import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { StoreContext } from "../stores/RootStore";
import { PresentationMode } from "./PresentationMode";

function makeMockStores() {
    const mockImage = {
        hash: "abc12345",
        device_id: "dev1",
        original_path: "/photos/pic.jpg",
        name: "pic.jpg",
        created_at: "2026-08-15T12:00:00Z",
        media_type: "image" as const,
        place: "High Tatras",
        thumbnailUrl: "/api/thumb/abc12345",
        starred: false,
    };

    const mediaStore = {
        fetchRandomImage: vi.fn().mockResolvedValue(mockImage),
    };

    const uiStore = {
        isFullscreen: false,
        setIsFullscreen: vi.fn(),
    };

    const labelStore = {
        labels: [
            { id: 1, name: "Vacation" },
            { id: 2, name: "Family" },
        ],
        fetchLabels: vi.fn(),
    };

    return { mediaStore, uiStore, labelStore, mockImage };
}

function renderPresentation(stores: ReturnType<typeof makeMockStores>) {
    return render(
        <StoreContext.Provider value={stores as never}>
            <PresentationMode />
        </StoreContext.Provider>
    );
}

describe("PresentationMode", () => {
    beforeEach(() => {
        vi.clearAllMocks();
        localStorage.clear();
    });

    afterEach(() => {
        localStorage.clear();
    });

    it("renders loading indicator initially and then loads image", async () => {
        const stores = makeMockStores();
        renderPresentation(stores);

        expect(screen.getByText(/Loading Presentation.../i)).toBeTruthy();
        expect(stores.mediaStore.fetchRandomImage).toHaveBeenCalled();

        const location = await screen.findByText("High Tatras");
        expect(location).toBeTruthy();
    });

    it("shows empty / error state if no images are found", async () => {
        const stores = makeMockStores();
        stores.mediaStore.fetchRandomImage.mockResolvedValue(null);
        renderPresentation(stores);

        const errorMsg = await screen.findByText(/No images found for these settings/i);
        expect(errorMsg).toBeTruthy();
        expect(screen.getByRole("button", { name: /Clear All Filters/i })).toBeTruthy();
    });

    it("reads default showTime (15s) and zoomSpeed ('normal') when localStorage is empty", async () => {
        const stores = makeMockStores();
        renderPresentation(stores);

        await screen.findByText("High Tatras");
        expect(screen.getByText("15s")).toBeTruthy();

        const user = userEvent.setup();
        const settingsBtn = screen.getByTitle("Presentation Settings");
        await user.click(settingsBtn);

        expect(screen.getByText("Presentation Settings")).toBeTruthy();
        expect(screen.getByText("15S SLIDES")).toBeTruthy();
        expect(screen.getByText("ZOOM: normal")).toBeTruthy();
    });

    it("reads custom showTime and zoomSpeed from localStorage", async () => {
        localStorage.setItem("present.showTime", JSON.stringify(30));
        localStorage.setItem("present.zoomSpeed", JSON.stringify("fast"));

        const stores = makeMockStores();
        renderPresentation(stores);

        await screen.findByText("High Tatras");
        expect(screen.getByText("30s")).toBeTruthy();

        const user = userEvent.setup();
        await user.click(screen.getByTitle("Presentation Settings"));

        expect(screen.getByText("30S SLIDES")).toBeTruthy();
        expect(screen.getByText("ZOOM: fast")).toBeTruthy();
    });

    it("allows changing slide duration via preset buttons and persists to localStorage", async () => {
        const stores = makeMockStores();
        renderPresentation(stores);
        await screen.findByText("High Tatras");

        const user = userEvent.setup();
        await user.click(screen.getByTitle("Presentation Settings"));

        const preset5s = screen.getByRole("button", { name: "5s" });
        await user.click(preset5s);

        expect(localStorage.getItem("present.showTime")).toBe("5");
        expect(screen.getByText("5S SLIDES")).toBeTruthy();
    });

    it("allows changing slide duration via range slider", async () => {
        const stores = makeMockStores();
        renderPresentation(stores);
        await screen.findByText("High Tatras");

        const user = userEvent.setup();
        await user.click(screen.getByTitle("Presentation Settings"));

        const slider = screen.getByLabelText("Slide Duration");
        fireEvent.change(slider, { target: { value: "10" } });

        expect(localStorage.getItem("present.showTime")).toBe("10");
        expect(screen.getByText("10S SLIDES")).toBeTruthy();
    });

    it("allows changing zoom speed and updates image animation class and duration", async () => {
        const stores = makeMockStores();
        renderPresentation(stores);
        await screen.findByText("High Tatras");

        const user = userEvent.setup();
        await user.click(screen.getByTitle("Presentation Settings"));

        // Select 'off'
        const offBtn = screen.getByRole("radio", { name: /^off$/i });
        expect(offBtn.getAttribute("aria-checked")).toBe("false");
        await user.click(offBtn);
        expect(localStorage.getItem("present.zoomSpeed")).toBe('"off"');
        expect(screen.getByText("ZOOM: off")).toBeTruthy();
        expect(offBtn.getAttribute("aria-checked")).toBe("true");

        const img = screen.getByAltText("pic.jpg");
        expect(img.style.animationDuration).toBe("");

        // Select 'slow'
        const slowBtn = screen.getByRole("radio", { name: /^slow$/i });
        await user.click(slowBtn);
        expect(localStorage.getItem("present.zoomSpeed")).toBe('"slow"');
        expect(screen.getByText("ZOOM: slow")).toBeTruthy();
        expect(slowBtn.getAttribute("aria-checked")).toBe("true");
        expect(img.style.animationDuration).toBe("15s");
        expect(img.className).toMatch(/animate-zoom-(in|out)-slow/);

        // Select 'fast'
        const fastBtn = screen.getByRole("radio", { name: /^fast$/i });
        await user.click(fastBtn);
        expect(localStorage.getItem("present.zoomSpeed")).toBe('"fast"');
        expect(screen.getByText("ZOOM: fast")).toBeTruthy();
        expect(fastBtn.getAttribute("aria-checked")).toBe("true");
        expect(img.className).toMatch(/animate-zoom-(in|out)-fast/);
    });

    it("toggles pause and resume and updates animationPlayState", async () => {
        const stores = makeMockStores();
        renderPresentation(stores);
        await screen.findByText("High Tatras");

        const img = screen.getByAltText("pic.jpg");
        expect(img.style.animationPlayState).toBe("running");

        const user = userEvent.setup();
        const pauseBtn = screen.getByTitle("Pause");
        await user.click(pauseBtn);

        expect(screen.getByTitle("Resume")).toBeTruthy();
        expect(img.style.animationPlayState).toBe("paused");

        await user.click(screen.getByTitle("Resume"));
        expect(screen.getByTitle("Pause")).toBeTruthy();
        expect(img.style.animationPlayState).toBe("running");
    });

    it("toggles show information overlay", async () => {
        const stores = makeMockStores();
        renderPresentation(stores);
        await screen.findByText("High Tatras");

        const user = userEvent.setup();
        const infoBtn = screen.getByTitle("Hide Info");
        await user.click(infoBtn);
        expect(localStorage.getItem("present.showInfo")).toBe("false");

        await user.click(screen.getByTitle("Show Info"));
        expect(localStorage.getItem("present.showInfo")).toBe("true");
    });

    it("does not exit fullscreen when toggling filter settings", async () => {
        const stores = makeMockStores();
        stores.uiStore.isFullscreen = true;
        renderPresentation(stores);
        await screen.findByText("High Tatras");

        const user = userEvent.setup();
        await user.click(screen.getByTitle("Presentation Settings"));

        const starredLabel = screen.getByText("Only Starred Images");
        await user.click(starredLabel);

        // Filter change triggers re-render/fetch, but must not call setIsFullscreen(false)
        expect(stores.uiStore.setIsFullscreen).not.toHaveBeenCalledWith(false);
    });

    it("toggles settings closed when clicking settings button again", async () => {
        const stores = makeMockStores();
        renderPresentation(stores);
        await screen.findByText("High Tatras");

        const user = userEvent.setup();
        const settingsBtn = screen.getByTitle("Presentation Settings");
        await user.click(settingsBtn);
        expect(screen.getByText("Presentation Settings")).toBeTruthy();

        await user.click(settingsBtn);
        expect(screen.queryByText("Presentation Settings")).toBeNull();
    });

    it("falls back to default settings on invalid or corrupted localStorage", async () => {
        localStorage.setItem("present.showTime", "invalid-json{");
        localStorage.setItem("present.zoomSpeed", JSON.stringify("ultra-fast"));

        const stores = makeMockStores();
        renderPresentation(stores);
        await screen.findByText("High Tatras");

        expect(screen.getByText("15s")).toBeTruthy();

        const user = userEvent.setup();
        await user.click(screen.getByTitle("Presentation Settings"));
        expect(screen.getByText("ZOOM: normal")).toBeTruthy();
    });
});
