import { describe, it, expect, beforeEach, afterEach } from "vitest";
import type { AxiosRequestConfig, AxiosResponse } from "axios";
import api from "./axiosConfig";

describe("axiosConfig", () => {
    const originalLocation = window.location;

    function setMockLocation(pathname: string, href = `http://localhost${pathname}`) {
        Object.defineProperty(window, "location", {
            configurable: true,
            writable: true,
            value: { pathname, href },
        });
    }

    beforeEach(() => {
        setMockLocation("/media");
    });

    afterEach(() => {
        Object.defineProperty(window, "location", {
            configurable: true,
            writable: true,
            value: originalLocation,
        });
    });

    it("configures baseURL=/api and withCredentials=true", () => {
        expect(api.defaults.baseURL).toBe("/api");
        expect(api.defaults.withCredentials).toBe(true);
    });

    it("passes successful responses through unchanged", async () => {
        const res = await api.get("/image_thumbnails", {
            adapter: async (config): Promise<AxiosResponse> => ({
                data: { ok: true },
                status: 200,
                statusText: "OK",
                headers: {},
                config,
            }),
        });

        expect(res.status).toBe(200);
        expect(res.data).toEqual({ ok: true });
        expect(window.location.href).toBe("http://localhost/media");
    });

    it("propagates request interceptor rejections", async () => {
        const handlers = (
            api.interceptors.request as unknown as {
                handlers: Array<{ rejected?: (err: unknown) => Promise<never> }>;
            }
        ).handlers;
        const rejected = handlers[0]?.rejected;
        expect(rejected).toBeDefined();
        await expect(rejected!(new Error("request boom"))).rejects.toThrow("request boom");
    });

    it("redirects to /login on 401 for protected endpoints", async () => {
        const make401Adapter = (config: AxiosRequestConfig) =>
            Promise.reject({
                config,
                response: { status: 401, data: { error: "unauthorized" } },
            });

        await expect(
            api.get("/image_thumbnails", { adapter: make401Adapter })
        ).rejects.toMatchObject({ response: { status: 401 } });

        expect(window.location.href).toBe("/login");
    });

    it("does not redirect on 401 for user-login, /auth/me, or when already on /login", async () => {
        const make401Adapter = (config: AxiosRequestConfig) =>
            Promise.reject({
                config,
                response: { status: 401 },
            });

        await expect(api.post("/user-login", {}, { adapter: make401Adapter })).rejects.toBeDefined();
        expect(window.location.href).toBe("http://localhost/media");

        await expect(api.get("/auth/me", { adapter: make401Adapter })).rejects.toBeDefined();
        expect(window.location.href).toBe("http://localhost/media");

        setMockLocation("/login", "http://localhost/login");
        await expect(api.get("/image_thumbnails", { adapter: make401Adapter })).rejects.toBeDefined();
        expect(window.location.href).toBe("http://localhost/login");
    });

    it("rejects non-401 errors without redirecting", async () => {
        await expect(
            api.get("/image_thumbnails", {
                adapter: async (config) =>
                    Promise.reject({
                        config,
                        response: { status: 500, data: "server error" },
                    }),
            })
        ).rejects.toMatchObject({ response: { status: 500 } });

        expect(window.location.href).toBe("http://localhost/media");
    });
});
