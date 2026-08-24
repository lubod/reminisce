import { describe, it, expect, vi, beforeEach } from "vitest";
import { AxiosError } from "axios";
import { StatsStore } from "./StatsStore";
import type { RootStore } from "./RootStore";

const mocks = vi.hoisted(() => {
    const routes: Record<string, () => unknown> = {};
    const get = vi.fn(async (url: string) => {
        const fn = routes[url];
        if (!fn) return { data: {}, status: 200 };
        return fn();
    });
    const put = vi.fn(async (url: string) => {
        const fn = routes["PUT " + url];
        if (!fn) return { data: {}, status: 200 };
        return fn();
    });
    const post = vi.fn(async (url: string) => {
        const fn = routes["POST " + url];
        if (!fn) return { data: {}, status: 200 };
        return fn();
    });
    const del = vi.fn(async (url: string) => {
        const fn = routes["DELETE " + url];
        if (!fn) return { data: {}, status: 200 };
        return fn();
    });
    const clear = () => { for (const k of Object.keys(routes)) delete routes[k]; };
    return { api: { get, put, post, delete: del }, routes, clear };
});

vi.mock("../api/axiosConfig", () => ({ __esModule: true, default: mocks.api }));

function verification(over: Partial<{ verified_files: number; degraded_files: number; failed_files: number; missing_files: number; total_files: number }> = {}) {
    return {
        total_files: 0,
        verified_files: 0,
        degraded_files: 0,
        failed_files: 0,
        missing_files: 0,
        files: [],
        ...over,
    };
}

function makeStore(): { store: StatsStore; errors: string[]; successes: string[] } {
    const errors: string[] = [];
    const successes: string[] = [];
    const mockRoot = {
        uiStore: {
            setError: (m: string) => errors.push(m),
            setSuccess: (m: string) => successes.push(m),
            setLoading: () => {},
        },
    } as unknown as RootStore;
    return { store: new StatsStore(mockRoot), errors, successes };
}

const emptyP2PStatus = (over: Partial<import("./StatsStore").P2PBackupStatus> = {}) => ({
    local_peer_id: "p1",
    is_healthy: true,
    health_status: "healthy" as const,
    active_peers: 2,
    total_shards_stored: 10,
    ok_files: 4,
    degraded_files: 0,
    failed_files: 0,
    missing_files: 0,
    pending_images: 0,
    pending_videos: 0,
    db_backups_count: 1,
    db_backups_total_bytes: 100,
    db_backups_latest_at: null,
    ...over,
});

describe("StatsStore", () => {
    beforeEach(() => mocks.clear());

    it("fetchStats populates stats and clears loading", async () => {
        mocks.routes["/stats"] = () => ({ data: { total_images: 5, total_videos: 2 } });
        const { store } = makeStore();
        await store.fetchStats();
        expect(store.stats?.total_images).toBe(5);
        expect(store.isLoading).toBe(false);
    });

    it("fetchStats failure surfaces an error", async () => {
        mocks.routes["/stats"] = () => { throw new Error("x"); };
        const { store, errors } = makeStore();
        await store.fetchStats();
        expect(errors).toContain("Failed to load dashboard statistics.");
        expect(store.isLoading).toBe(false);
    });

    it("fetchPoolStats populates pool stats (and tolerates failure)", async () => {
        mocks.routes["/pool-stats"] = () => ({ data: { main_pool: { size: 1, available: 2, max_size: 10, utilization_percent: 30 } } });
        const { store } = makeStore();
        await store.fetchPoolStats();
        expect(store.poolStats?.main_pool.size).toBe(1);
        expect(store.isPoolStatsLoading).toBe(false);

        mocks.routes["/pool-stats"] = () => { throw new Error("x"); };
        await store.fetchPoolStats();
        expect(store.isPoolStatsLoading).toBe(false);
    });

    it("fetchGeoDbStats populates geodb stats (and tolerates failure)", async () => {
        mocks.routes["/geodb-stats"] = () => ({ data: { countries: 42 } });
        const { store } = makeStore();
        await store.fetchGeoDbStats();
        expect(store.geoDbStats?.countries).toBe(42);
        expect(store.isGeoDbStatsLoading).toBe(false);

        mocks.routes["/geodb-stats"] = () => { throw new Error("x"); };
        await store.fetchGeoDbStats();
        expect(store.isGeoDbStatsLoading).toBe(false);
    });

    it("fetchAiSettings populates AI settings (and tolerates failure)", async () => {
        mocks.routes["/ai-settings"] = () => ({ data: { enable_ai_descriptions: true } });
        const { store } = makeStore();
        await store.fetchAiSettings();
        expect(store.aiSettings?.enable_ai_descriptions).toBe(true);
        expect(store.isAiSettingsLoading).toBe(false);

        mocks.routes["/ai-settings"] = () => { throw new Error("x"); };
        await store.fetchAiSettings();
        expect(store.isAiSettingsLoading).toBe(false);
    });

    it("updateAiSettings updates stored settings on success", async () => {
        mocks.routes["PUT /ai-settings"] = () => ({ data: { enable_ai_descriptions: false } });
        const { store } = makeStore();
        await store.updateAiSettings({ enable_ai_descriptions: false });
        expect(store.aiSettings?.enable_ai_descriptions).toBe(false);
    });

    it("updateAiSettings failure surfaces an error and rethrows", async () => {
        mocks.routes["PUT /ai-settings"] = () => { throw new Error("x"); };
        const { store, errors } = makeStore();
        await expect(store.updateAiSettings({})).rejects.toThrow();
        expect(errors).toContain("Failed to update AI settings");
    });

    it("fetchSystemStats populates system stats (and tolerates failure)", async () => {
        mocks.routes["/system-stats"] = () => ({ data: { cpu_usage_percent: 12, memory_total_gb: 16 } });
        const { store } = makeStore();
        await store.fetchSystemStats();
        expect(store.systemStats?.cpu_usage_percent).toBe(12);
        expect(store.isSystemStatsLoading).toBe(false);

        mocks.routes["/system-stats"] = () => { throw new Error("x"); };
        await store.fetchSystemStats();
        expect(store.isSystemStatsLoading).toBe(false);
    });

    it("fetchP2PDaemonStatus populates daemon status", async () => {
        mocks.routes["/p2p-daemon-status"] = () => ({ data: { is_healthy: true, active_peers: 1 } });
        const { store } = makeStore();
        await store.fetchP2PDaemonStatus();
        expect(store.p2pDaemonStatus?.is_healthy).toBe(true);
        expect(store.isP2PDaemonStatusLoading).toBe(false);
    });

    it("fetchP2PDaemonStatus failure nulls the status", async () => {
        mocks.routes["/p2p-daemon-status"] = () => { throw new Error("x"); };
        const { store } = makeStore();
        store.p2pDaemonStatus = { is_healthy: true } as never;
        await store.fetchP2PDaemonStatus();
        expect(store.p2pDaemonStatus).toBeNull();
        expect(store.isP2PDaemonStatusLoading).toBe(false);
    });

    it("fetchP2PBackupStatus populates backup status (and tolerates failure)", async () => {
        mocks.routes["/p2p/backup/status"] = () => ({ data: emptyP2PStatus({ active_peers: 3 }) });
        const { store } = makeStore();
        await store.fetchP2PBackupStatus();
        expect(store.p2pBackupStatus?.active_peers).toBe(3);
        expect(store.isP2PBackupStatsLoading).toBe(false);

        mocks.routes["/p2p/backup/status"] = () => { throw new Error("x"); };
        await store.fetchP2PBackupStatus();
        expect(store.isP2PBackupStatsLoading).toBe(false);
    });

    it("fetchDiscoveredPeers populates peers (and tolerates failure)", async () => {
        mocks.routes["/p2p-discovered-peers"] = () => ({ data: { peer_count: 1, peers: [{ peer_id: "a" }] } });
        const { store } = makeStore();
        await store.fetchDiscoveredPeers();
        expect(store.discoveredPeers).toHaveLength(1);
        expect(store.isDiscoveredPeersLoading).toBe(false);

        mocks.routes["/p2p-discovered-peers"] = () => { throw new Error("x"); };
        await store.fetchDiscoveredPeers();
        expect(store.isDiscoveredPeersLoading).toBe(false);
    });

    it("removeNode deletes the node and filters it from the list", async () => {
        mocks.routes["/p2p-discovered-peers"] = () => ({ data: { peer_count: 2, peers: [{ peer_id: "a" }, { peer_id: "b" }] } });
        mocks.routes["DELETE /p2p/nodes/a"] = () => ({ data: {} });
        const { store, successes } = makeStore();
        await store.fetchDiscoveredPeers();

        await store.removeNode("a");
        expect(mocks.api.delete).toHaveBeenCalledWith("/p2p/nodes/a");
        expect(store.discoveredPeers.map(p => p.peer_id)).toEqual(["b"]);
        expect(successes.join()).toContain("Node removed and shards deleted.");
    });

    it("removeNode failure surfaces an error and rethrows", async () => {
        mocks.routes["DELETE /p2p/nodes/a"] = () => { throw new Error("boom"); };
        const { store, errors } = makeStore();
        await expect(store.removeNode("a")).rejects.toThrow("boom");
        expect(errors).toContain("Failed to remove node");
    });

    it("removeNode extracts the server error message on HTTP failure", async () => {
        const err = new AxiosError(
            "x", undefined, undefined, undefined,
            { status: 400, data: { error: "node busy" }, statusText: "", headers: {}, config: {} } as unknown as import("axios").AxiosResponse,
        );
        mocks.routes["DELETE /p2p/nodes/a"] = () => { throw err; };
        const { store, errors } = makeStore();
        await expect(store.removeNode("a")).rejects.toBe(err);
        expect(errors).toContain("node busy");
    });

    it("forceRebalance posts and reports success", async () => {
        mocks.routes["POST /p2p/backup/rebalance"] = () => ({ data: {} });
        const { store, successes } = makeStore();
        await store.forceRebalance();
        expect(mocks.api.post).toHaveBeenCalledWith("/p2p/backup/rebalance");
        expect(successes.join()).toContain("Rebalance triggered");
    });

    it("fetchServiceHealth uses the un-proxied /health endpoint", async () => {
        mocks.routes["/health"] = () => ({ data: { status: "ok" } });
        const { store } = makeStore();
        await store.fetchServiceHealth();
        expect(store.serviceHealth?.status).toBe("ok");
        expect(mocks.api.get).toHaveBeenCalledWith("/health", expect.objectContaining({ baseURL: "" }));
    });

    it("fetchServiceHealth treats a 200 non-JSON payload as offline", async () => {
        // A reverse proxy can answer 200 with an HTML error page.
        mocks.routes["/health"] = () => ({
            data: "<html>502 Bad Gateway</html>",
            status: 200,
            headers: { "content-type": "text/html" },
        });
        const { store } = makeStore();
        await store.fetchServiceHealth();
        expect(store.serviceHealth?.status).toBe("offline");
        expect(store.serviceHealth?.database).toBe("disconnected");
    });

    it("fetchServiceHealth falls back to an offline snapshot on failure", async () => {
        mocks.routes["/health"] = () => { throw new Error("x"); };
        const { store } = makeStore();
        await store.fetchServiceHealth();
        expect(store.serviceHealth?.status).toBe("offline");
        expect(store.serviceHealth?.database).toBe("disconnected");
    });

    it("fetchAllStats runs every collector without throwing", async () => {
        mocks.routes["/stats"] = () => ({ data: { total_images: 1 } });
        mocks.routes["/pool-stats"] = () => ({ data: {} });
        mocks.routes["/geodb-stats"] = () => ({ data: {} });
        mocks.routes["/ai-settings"] = () => ({ data: {} });
        mocks.routes["/system-stats"] = () => ({ data: {} });
        mocks.routes["/health"] = () => ({ data: { status: "ok" } });
        mocks.routes["/p2p-daemon-status"] = () => ({ data: {} });
        mocks.routes["/p2p-discovered-peers"] = () => ({ data: { peers: [] } });
        mocks.routes["/p2p/backup/status"] = () => ({ data: emptyP2PStatus() });
        const { store } = makeStore();

        await store.fetchAllStats();
        expect(store.stats?.total_images).toBe(1);
        expect(store.discoveredPeers).toEqual([]);
    });

    describe("verifyP2PBackup", () => {
        it("reports failed/missing files through uiStore.setError", async () => {
            mocks.routes["/p2p/backup/verify"] = () => ({ data: verification({ failed_files: 2, missing_files: 3, total_files: 5 }) });
            const { store, errors } = makeStore();
            await store.verifyP2PBackup();
            expect(store.verificationResult?.failed_files).toBe(2);
            expect(errors.join()).toContain("2 failed, 3 missing");
        });

        it("reports degraded files through setSuccess", async () => {
            mocks.routes["/p2p/backup/verify"] = () => ({ data: verification({ verified_files: 10, degraded_files: 2, total_files: 12 }) });
            const { store, successes } = makeStore();
            await store.verifyP2PBackup();
            expect(successes.join()).toContain("Verify OK: 10 full, 2 degraded");
        });

        it("reports a fully healthy verification", async () => {
            mocks.routes["/p2p/backup/verify"] = () => ({ data: verification({ verified_files: 42, total_files: 42 }) });
            const { store, successes } = makeStore();
            await store.verifyP2PBackup();
            expect(successes.join()).toContain("All 42 files verified successfully!");
        });

        it("does not toast when there is nothing to verify", async () => {
            mocks.routes["/p2p/backup/verify"] = () => ({ data: verification({ total_files: 0 }) });
            const { store, errors, successes } = makeStore();
            await store.verifyP2PBackup();
            expect(errors).toHaveLength(0);
            expect(successes).toHaveLength(0);
            expect(store.verificationResult?.total_files).toBe(0);
        });

        it("rethrows and surfaces the server error message", async () => {
            const err = new AxiosError(
                "x",
                undefined,
                undefined,
                undefined,
                { status: 500, data: { error: "DB down" }, statusText: "", headers: {}, config: {} } as unknown as import("axios").AxiosResponse,
            );
            mocks.routes["/p2p/backup/verify"] = () => { throw err; };
            const { store, errors } = makeStore();
            await expect(store.verifyP2PBackup()).rejects.toBe(err);
            expect(errors).toContain("DB down");
        });
    });
});
