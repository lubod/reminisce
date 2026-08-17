import { describe, it, expect, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { P2PRebalanceProgress } from "./P2PRebalanceProgress";
import { StoreContext, RootStore } from "../../stores/RootStore";

describe("P2PRebalanceProgress", () => {
    let store: RootStore;

    beforeEach(() => {
        store = new RootStore();
    });

    it("renders optimal state when progress is 100%", () => {
        store.statsStore.p2pBackupStatus = {
            local_peer_id: "test-node",
            is_healthy: true,
            health_status: "healthy",
            active_peers: 3,
            total_shards_stored: 300000,
            ok_files: 60000,
            degraded_files: 0,
            failed_files: 0,
            missing_files: 0,
            pending_images: 0,
            pending_videos: 0,
            db_backups_count: 5,
            db_backups_total_bytes: 1000000,
            db_backups_latest_at: null,
            rebalance: {
                is_active: false,
                total_files: 60000,
                balanced_files: 60000,
                unbalanced_files: 0,
                progress_percent: 100.0,
                target_nodes: 3,
            },
        };

        render(
            <StoreContext.Provider value={store}>
                <P2PRebalanceProgress />
            </StoreContext.Provider>
        );

        expect(screen.getByText("Mesh Shard Distribution & Rebalance")).toBeDefined();
        expect(screen.getByText("OPTIMAL (100%)")).toBeDefined();
        expect(screen.getByText("100.0%")).toBeDefined();
        expect(screen.getAllByText("60,000").length).toBeGreaterThanOrEqual(1);
    });

    it("renders active rebalancing state with percentage and pending files", () => {
        store.statsStore.p2pBackupStatus = {
            local_peer_id: "test-node",
            is_healthy: true,
            health_status: "healthy",
            active_peers: 3,
            total_shards_stored: 300000,
            ok_files: 60000,
            degraded_files: 0,
            failed_files: 0,
            missing_files: 0,
            pending_images: 0,
            pending_videos: 0,
            db_backups_count: 5,
            db_backups_total_bytes: 1000000,
            db_backups_latest_at: null,
            rebalance: {
                is_active: true,
                total_files: 60000,
                balanced_files: 3000,
                unbalanced_files: 57000,
                progress_percent: 5.0,
                target_nodes: 3,
            },
        };

        store.statsStore.discoveredPeers = [
            { peer_id: "node1", is_active: true, last_seen: "2026-08-17", shard_count: 290000, public_addr: "192.168.1.155:5050" },
            { peer_id: "node2", is_active: true, last_seen: "2026-08-17", shard_count: 5000, public_addr: "192.168.1.155:5051" },
            { peer_id: "node3", is_active: true, last_seen: "2026-08-17", shard_count: 5000, public_addr: "192.168.1.155:5052" },
        ];

        render(
            <StoreContext.Provider value={store}>
                <P2PRebalanceProgress />
            </StoreContext.Provider>
        );

        expect(screen.getByText("REBALANCING (5.0%)")).toBeDefined();
        expect(screen.getByText("5.0%")).toBeDefined();
        expect(screen.getByText("3,000")).toBeDefined();
        expect(screen.getByText("57,000")).toBeDefined();
        expect(screen.getByText("192.168.1.155:5050")).toBeDefined();
    });
});
