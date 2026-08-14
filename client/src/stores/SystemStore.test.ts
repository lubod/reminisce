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

    it("loads pipeline timings", async () => {
        mockedGet.mockResolvedValueOnce({
            data: {
                workers: [{ id: "embedding", name: "Embedding", count: 5, mean_ms: 120, p50_ms: 100, p90_ms: 200, p95_ms: 250, p99_ms: 300 }],
                http: { total: 100, per_second: 0.4, status: { http_2xx: 90, http_3xx: 0, http_4xx: 8, http_5xx: 2 }, duration_ms: { id: "http", name: "HTTP", count: 100, mean_ms: 25, p50_ms: 10, p90_ms: 40, p95_ms: 50, p99_ms: 80 } },
                db_query_ms: { id: "db", name: "DB", count: 10, mean_ms: 5, p50_ms: 3, p90_ms: 8, p95_ms: 9, p99_ms: 10 },
            },
        });
        const s = makeStore();
        await s.loadPipeline();
        expect(s.pipeline?.workers[0].p95_ms).toBe(250);
        expect(s.pipeline?.http.status.http_5xx).toBe(2);
        expect(s.pipeline?.db_query_ms.p99_ms).toBe(10);
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
        // Deduped: the second refreshAll did not re-run the 9 loaders.
        expect(mockedGet).toHaveBeenCalledTimes(10);
    });

    it("loads series and changes range", async () => {
        mockedGet.mockResolvedValueOnce({
            data: { range: "1d", series: [{ name: "system_cpu_percent", unit: "%", points: [{ t: 1, v: 12.5 }] }] },
        });
        const s = makeStore();
        await s.loadSeries();
        expect(mockedGet).toHaveBeenCalledWith("/admin/series", { params: { range: "1d" } });
        expect(s.series[0].name).toBe("system_cpu_percent");
        expect(s.series[0].points[0].v).toBe(12.5);

        mockedGet.mockResolvedValueOnce({ data: { range: "30d", series: [] } });
        s.setRange("30d");
        expect(s.seriesRange).toBe("30d");
        expect(mockedGet).toHaveBeenLastCalledWith("/admin/series", { params: { range: "30d" } });
        // same range is a no-op
        s.setRange("30d");
        expect(mockedGet).toHaveBeenCalledTimes(2);
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

    it("loads AI models runtime status", async () => {
        mockedGet.mockResolvedValueOnce({
            data: {
                status: "healthy",
                device: "cuda:0",
                models: [
                    { id: "siglip2", name: "SigLIP2", model_id: "google/siglip2", task: "Search", loaded: true, status: "active", dim: 1152 },
                ],
            },
        });
        const s = makeStore();
        await s.loadAiModels();
        expect(mockedGet).toHaveBeenCalledWith("/admin/ai-models");
        expect(s.aiModels?.status).toBe("healthy");
        expect(s.aiModels?.models[0].name).toBe("SigLIP2");
    });

});
