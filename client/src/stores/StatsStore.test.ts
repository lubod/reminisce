import { describe, it, expect, vi } from "vitest";
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
    const clear = () => { for (const k of Object.keys(routes)) delete routes[k]; };
    return { api: { get }, routes, clear };
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

describe("StatsStore.verifyP2PBackup", () => {
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
            {
                status: 500,
                data: { error: "DB down" },
                statusText: "",
                headers: {},
                config: {},
            } as unknown as import("axios").AxiosResponse,
        );
        mocks.routes["/p2p/backup/verify"] = () => { throw err; };
        const { store, errors } = makeStore();
        await expect(store.verifyP2PBackup()).rejects.toBe(err);
        expect(errors).toContain("DB down");
    });
});
