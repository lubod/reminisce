import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("../api/axiosConfig", () => ({
    __esModule: true,
    default: { get: vi.fn() },
}));

import api from "../api/axiosConfig";
import { SystemStore } from "./SystemStore";

const mockedGet = vi.mocked(api.get);

// Minimal RootStore stand-in (SystemStore only uses `this.root` indirectly).
class MiniRoot {}

const logEntry = (over: Partial<{ timestamp: number; level: string; target: string; message: string }> = {}) => ({
    timestamp: 1,
    level: "INFO",
    target: "app",
    message: "hello",
    fields: {},
    ...over,
});

const backupStatus = {
    is_healthy: true,
    health_status: "healthy",
    active_peers: 5,
    ok_files: 100,
    degraded_files: 0,
    missing_files: 0,
    pending_images: 0,
    pending_videos: 0,
    db_backups_count: 3,
    db_backups_latest_at: "2026-08-09T00:00:00Z",
};

beforeEach(() => {
    mockedGet.mockReset();
});

function makeStore() {
    const root = new (class extends MiniRoot {})();
    return new SystemStore(root as never);
}

describe("SystemStore", () => {
    it("loads logs", async () => {
        mockedGet.mockResolvedValueOnce({
            data: { entries: [logEntry({ level: "ERROR" })], source: "ring" },
        });
        const s = makeStore();
        await s.loadLogs("error", 100);
        expect(mockedGet).toHaveBeenCalledWith("/admin/logs", { params: { level: "error", limit: 100 } });
        expect(s.logs).toHaveLength(1);
        expect(s.logs[0].level).toBe("ERROR");
        expect(s.logSource).toBe("ring");
        expect(s.lastError).toBeNull();
    });

    it("loads errors with counts", async () => {
        mockedGet.mockResolvedValueOnce({
            data: {
                entries: [logEntry({ level: "ERROR" })],
                count_5m: { error: 1, warn: 2, panic: 0 },
            },
        });
        const s = makeStore();
        await s.loadErrors();
        expect(s.errors).toHaveLength(1);
        expect(s.errorCounts).toEqual({ error: 1, warn: 2, panic: 0 });
    });

    it("loads alerts, system, pool, backup and gpu", async () => {
        mockedGet.mockResolvedValueOnce({ data: { alerts: [{ id: "X", severity: "warning", status: "firing" }] } });
        mockedGet.mockResolvedValueOnce({ data: { cpu_usage_percent: 12 } });
        mockedGet.mockResolvedValueOnce({ data: { main_pool: { size: 8, available: 6, max_size: 8, utilization_percent: 25 } } });
        mockedGet.mockResolvedValueOnce({ data: backupStatus });
        mockedGet.mockResolvedValueOnce({ data: { available: true, cards: [{ gpu: "0", utilization_percent: 50 }] } });

        const s = makeStore();
        await Promise.all([s.loadAlerts(), s.loadSystem(), s.loadPool(), s.loadBackup(), s.loadGpu()]);

        expect(s.alerts[0].id).toBe("X");
        expect(s.system?.cpu_usage_percent).toBe(12);
        expect(s.pool?.main_pool.utilization_percent).toBe(25);
        expect(s.backup?.health_status).toBe("healthy");
        expect(s.gpu?.cards[0].utilization_percent).toBe(50);
        expect(s.lastError).toBeNull();
    });

    it("records errors without throwing", async () => {
        mockedGet.mockRejectedValueOnce(new Error("boom"));
        const s = makeStore();
        await s.loadAlerts();
        expect(s.lastError).toContain("boom");
        expect(s.alerts).toEqual([]);
    });

    it("is single-flight and toggles isLoading", async () => {
        mockedGet.mockResolvedValue({ data: { entries: [], source: "ring" } });
        const s = makeStore();
        const p1 = s.refreshAll();
        const p2 = s.refreshAll();
        await Promise.all([p1, p2]);
        expect(s.isLoading).toBe(false);
        // Deduped: the second refreshAll did not re-run the 7 loaders.
        expect(mockedGet).toHaveBeenCalledTimes(7);
    });

    it("startAutoRefresh polls on an interval and cleans up", async () => {
        vi.useFakeTimers();
        try {
            mockedGet.mockResolvedValue({ data: { entries: [], source: "ring" } });
            const s = makeStore();
            const cleanup = s.startAutoRefresh(1000);
            await vi.runOnlyPendingTimersAsync();
            expect(mockedGet).toHaveBeenCalled();
            cleanup();
            const calls = mockedGet.mock.calls.length;
            await vi.advanceTimersByTimeAsync(5000);
            expect(mockedGet.mock.calls.length).toBe(calls);
        } finally {
            vi.useRealTimers();
        }
    });
});