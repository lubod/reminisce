import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ErrorsViewer } from "./ErrorsViewer";
import { StoreContext } from "../../stores/RootStore";
import type { LogLine } from "../../stores/SystemStore";

describe("ErrorsViewer", () => {
    const mockErrors: LogLine[] = [
        {
            timestamp: Math.floor(Date.now() / 1000),
            level: "ERROR",
            target: "p2p_audit",
            message: "Repair failed for shard 123",
        },
        {
            timestamp: Math.floor(Date.now() / 1000),
            level: "ERROR",
            target: "p2p_audit",
            message: "Repair failed for shard 123", // Duplicate to test clustering
        },
        {
            timestamp: Math.floor(Date.now() / 1000),
            level: "WARN",
            target: "system",
            message: "High memory utilization warning",
        },
    ];

    function renderErrorsViewer(errors: LogLine[] = mockErrors) {
        const mockSystemStore = {
            errors,
            errorCounts: { error: 2, panic: 0, warn: 1 },
            fetchErrors: vi.fn(),
        };

        return render(
            <StoreContext.Provider value={{ systemStore: mockSystemStore } as never}>
                <ErrorsViewer />
            </StoreContext.Provider>
        );
    }

    it("renders error entries with deduplicated count and clustering", () => {
        renderErrorsViewer();

        // Deduplication shows 2× occurrences for repeated error
        expect(screen.getByText(/2× occurrences/)).toBeTruthy();
        expect(screen.getByText(/Repair failed for shard 123/)).toBeTruthy();
    });

    it("filters entries by search query", () => {
        renderErrorsViewer();

        const searchInput = screen.getByPlaceholderText(/search recent errors/i);
        fireEvent.change(searchInput, { target: { value: "memory" } });

        expect(screen.getByText(/High memory utilization warning/)).toBeTruthy();
        expect(screen.queryByText(/Repair failed/)).toBeNull();
    });

    it("filters entries by level pill selection", () => {
        renderErrorsViewer();

        const warnBtn = screen.getByRole("button", { name: "WARN" });
        fireEvent.click(warnBtn);

        expect(screen.getByText(/High memory utilization warning/)).toBeTruthy();
        expect(screen.queryByText(/Repair failed/)).toBeNull();
    });

    it("shows clean empty state when no error logs are present", () => {
        renderErrorsViewer([]);

        expect(screen.getByText(/System Operating Cleanly/i)).toBeTruthy();
    });
});
