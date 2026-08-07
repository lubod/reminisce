import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { ErrorBoundary } from "./ErrorBoundary";

function Bomb(): React.ReactElement {
    throw new Error("kaboom");
}

function Fine(): React.ReactElement {
    return <div>child content</div>;
}

// Suppress the expected React error logging caused by the thrown child.
beforeEach(() => {
    vi.spyOn(console, "error").mockImplementation(() => {});
});

afterEach(() => {
    (console.error as unknown as { mockRestore: () => void }).mockRestore();
});

describe("ErrorBoundary", () => {
    it("renders its children when nothing throws", () => {
        render(<ErrorBoundary><Fine /></ErrorBoundary>);
        expect(screen.getByText("child content")).toBeTruthy();
        expect(screen.queryByText("Something went wrong")).toBeNull();
    });

    it("renders the fallback UI when a child throws", () => {
        render(<ErrorBoundary><Bomb /></ErrorBoundary>);
        expect(screen.getByText("Something went wrong")).toBeTruthy();
        expect(screen.getByRole("button", { name: /reload/i })).toBeTruthy();
        expect(screen.queryByText("child content")).toBeNull();
    });
});
