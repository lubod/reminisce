import { describe, it, expect } from "vitest";
import { UIStore } from "./UIStore";
import type { RootStore } from "./RootStore";

function makeStore(): UIStore {
    const mockRoot = {} as unknown as RootStore;
    return new UIStore(mockRoot);
}

describe("UIStore", () => {
    it("tracks loading state", () => {
        const s = makeStore();
        expect(s.isLoading).toBe(false);
        s.setLoading(true);
        expect(s.isLoading).toBe(true);
        s.setLoading(false);
        expect(s.isLoading).toBe(false);
    });

    it("setError stores the error and clears any success", () => {
        const s = makeStore();
        s.setSuccess("all good");
        s.setError("boom");
        expect(s.error).toBe("boom");
        expect(s.success).toBeNull();
    });

    it("clearing the error leaves success untouched", () => {
        const s = makeStore();
        s.setSuccess("all good");
        s.setError(null);
        expect(s.error).toBeNull();
        expect(s.success).toBe("all good");
    });

    it("setSuccess stores the message and clears any error", () => {
        const s = makeStore();
        s.setError("boom");
        s.setSuccess("ok");
        expect(s.success).toBe("ok");
        expect(s.error).toBeNull();
    });

    it("clearing the success message leaves error untouched", () => {
        const s = makeStore();
        s.setError("boom");
        s.setSuccess(null);
        expect(s.success).toBeNull();
        expect(s.error).toBe("boom");
    });

    it("tracks fullscreen state", () => {
        const s = makeStore();
        expect(s.isFullscreen).toBe(false);
        s.setIsFullscreen(true);
        expect(s.isFullscreen).toBe(true);
    });
});
