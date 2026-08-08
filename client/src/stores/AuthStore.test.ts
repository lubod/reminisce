import { describe, it, expect, vi } from "vitest";
import { AxiosError } from "axios";
import { AuthStore } from "./AuthStore";
import type { RootStore } from "./RootStore";

const mocks = vi.hoisted(() => {
    const routes: Record<string, () => unknown> = {};
    const get = vi.fn(async (url: string) => {
        const fn = routes["GET " + url];
        if (!fn) return { data: {}, status: 200 };
        return fn();
    });
    const post = vi.fn(async (url: string) => {
        const fn = routes["POST " + url];
        if (!fn) return { data: {}, status: 200 };
        return fn();
    });
    const patch = vi.fn(async (url: string) => {
        const fn = routes["PATCH " + url];
        if (!fn) return { data: {}, status: 200 };
        return fn();
    });
    const del = vi.fn(async (url: string) => {
        const fn = routes["DELETE " + url];
        if (!fn) return { data: {}, status: 200 };
        return fn();
    });
    const clearRoutes = () => { for (const k of Object.keys(routes)) delete routes[k]; };
    return { api: { get, post, patch, delete: del }, routes, clearRoutes };
});

vi.mock("../api/axiosConfig", () => ({ __esModule: true, default: mocks.api }));

const meUser = {
    id: "550e8400-e29b-41d4-a716-446655440000",
    username: "test-user",
    role: "admin",
    access_token: "ACCESS",
    image_token: "IMAGE",
};

function axError(status: number, data: unknown): AxiosError {
    return new AxiosError(
        "Request failed",
        undefined,
        undefined,
        undefined,
        {
            status,
            data,
            statusText: "",
            headers: {},
            config: {},
        } as unknown as import("axios").AxiosResponse,
    );
}

function makeStore(): AuthStore {
    const mockRoot = { uiStore: { setError: () => {} } } as unknown as RootStore;
    return new AuthStore(mockRoot);
}

describe("AuthStore", () => {
    it("initialize loads the session when setup is complete", async () => {
        mocks.routes["GET /auth/setup-status"] = () => ({ data: { needs_setup: false } });
        mocks.routes["GET /auth/me"] = () => ({ data: meUser });
        const s = makeStore();

        await s.initialize();

        expect(s.initialized).toBe(true);
        expect(s.needsSetup).toBe(false);
        expect(s.isAuthenticated).toBe(true);
        expect(s.user?.username).toBe("test-user");
        expect(s.token).toBe("ACCESS");
        expect(s.imageToken).toBe("IMAGE");
    });

    it("initialize clears the session on a definitive 401", async () => {
        mocks.routes["GET /auth/setup-status"] = () => ({ data: { needs_setup: false } });
        mocks.routes["GET /auth/me"] = () => { throw axError(401, {}); };
        const s = makeStore();
        s.setToken("stale");
        s.setUser({ id: "x", username: "u", role: "admin" });

        await s.initialize();

        expect(s.isAuthenticated).toBe(false);
        expect(s.token).toBeNull();
        expect(s.user).toBeNull();
    });

    it("initialize does NOT log out on transient (5xx) failures", async () => {
        mocks.routes["GET /auth/setup-status"] = () => ({ data: { needs_setup: false } });
        mocks.routes["GET /auth/me"] = () => { throw axError(500, {}); };
        const s = makeStore();
        s.setToken("keep");
        s.setUser({ id: "x", username: "u", role: "admin" });

        await s.initialize();

        expect(s.isAuthenticated).toBe(true);
        expect(s.token).toBe("keep");
        expect(s.user?.username).toBe("u");
    });

    it("initialize enters setup mode when no users exist", async () => {
        mocks.routes["GET /auth/setup-status"] = () => ({ data: { needs_setup: true } });
        const s = makeStore();
        await s.initialize();
        expect(s.initialized).toBe(true);
        expect(s.needsSetup).toBe(true);
        expect(s.isAuthenticated).toBe(false);
    });

    it("login stores token and user on success", async () => {
        mocks.routes["POST /auth/user-login"] = () => ({
            data: { access_token: "A", image_token: "I", user: { id: "1", username: "admin", role: "admin" } },
        });
        const s = makeStore();
        const result = await s.login("admin", "pass");
        expect(result.success).toBe(true);
        expect(s.token).toBe("A");
        expect(s.imageToken).toBe("I");
        expect(s.user?.username).toBe("admin");
        expect(s.isAuthenticated).toBe(true);
    });

    it("login failure returns the server message and does not authenticate", async () => {
        mocks.routes["POST /auth/user-login"] = () => { throw axError(401, { message: "Invalid username or password" }); };
        const s = makeStore();
        const result = await s.login("admin", "bad");
        expect(result.success).toBe(false);
        if (!result.success) expect(result.error).toContain("Invalid username or password");
        expect(s.isAuthenticated).toBe(false);
    });

    it("setupAdmin marks setup complete on success", async () => {
        mocks.routes["POST /auth/setup"] = () => ({ data: { status: "ok" } });
        const s = makeStore();
        const result = await s.setupAdmin("admin", "pass1234");
        expect(result.success).toBe(true);
        expect(s.needsSetup).toBe(false);
    });

    it("setupAdmin returns the server error on failure", async () => {
        mocks.routes["POST /auth/setup"] = () => { throw axError(400, { message: "Password too short" }); };
        const s = makeStore();
        const result = await s.setupAdmin("admin", "x");
        expect(result.success).toBe(false);
        if (!result.success) expect(result.error).toContain("Password too short");
    });

    it("logout clears all session state even if the server call fails", async () => {
        mocks.api.post.mockImplementationOnce(async () => { throw new Error("network"); });
        const s = makeStore();
        s.setToken("A");
        s.setUser({ id: "1", username: "u", role: "admin" });
        await s.logout();
        expect(s.token).toBeNull();
        expect(s.imageToken).toBeNull();
        expect(s.user).toBeNull();
        expect(s.isAuthenticated).toBe(false);
    });

    it("login falls back to the plain error message for non-HTTP errors", async () => {
        mocks.api.post.mockImplementationOnce(async () => { throw new Error("socket hang up"); });
        const s = makeStore();
        const result = await s.login("admin", "pass");
        expect(result.success).toBe(false);
        if (!result.success) expect(result.error).toBe("socket hang up");
        expect(s.isAuthenticated).toBe(false);
    });

    it("initialize tolerates a failed setup-status check", async () => {
        mocks.routes["GET /auth/setup-status"] = () => { throw new Error("offline"); };
        const s = makeStore();
        await s.initialize();
        expect(s.initialized).toBe(true);
        expect(s.needsSetup).toBe(false);
    });

    it("checkSetupStatus records needsSetup and falls back on failure", async () => {
        mocks.routes["GET /auth/setup-status"] = () => ({ data: { needs_setup: true } });
        const s = makeStore();
        await s.checkSetupStatus();
        expect(s.needsSetup).toBe(true);

        mocks.routes["GET /auth/setup-status"] = () => { throw new Error("x"); };
        await s.checkSetupStatus();
        expect(s.needsSetup).toBe(false);
    });

    it("listUsers returns the managed users", async () => {
        mocks.routes["GET /users"] = () => ({ data: [{ id: "1", username: "admin", email: "", role: "admin", is_active: true, created_at: "2024", last_login_at: null }] });
        const s = makeStore();
        const users = await s.listUsers();
        expect(users).toHaveLength(1);
        expect(users[0].username).toBe("admin");
    });

    it("createUser succeeds and returns the server failure message", async () => {
        mocks.routes["POST /users"] = () => ({ data: {}, status: 201 });
        const s = makeStore();
        expect((await s.createUser("bob", "pw", "user")).success).toBe(true);

        mocks.routes["POST /users"] = () => { throw axError(400, { message: "Username taken" }); };
        const result = await s.createUser("bob", "pw", "user");
        expect(result.success).toBe(false);
        if (!result.success) expect(result.error).toBe("Username taken");

        mocks.routes["POST /users"] = () => { throw new Error("net"); };
        const result2 = await s.createUser("bob", "pw", "user");
        if (!result2.success) expect(result2.error).toBe("Failed to create user");
    });

    it("updateUser succeeds and returns the server failure message", async () => {
        mocks.routes["PATCH /users/1"] = () => ({ data: {}, status: 200 });
        const s = makeStore();
        expect((await s.updateUser("1", { role: "admin" })).success).toBe(true);
        expect(mocks.api.patch).toHaveBeenCalledWith("/users/1", { role: "admin" });

        mocks.routes["PATCH /users/1"] = () => { throw axError(403, { message: "Not allowed" }); };
        const result = await s.updateUser("1", { is_active: false });
        expect(result.success).toBe(false);
        if (!result.success) expect(result.error).toBe("Not allowed");
    });

    it("deleteUser succeeds and returns the server failure message", async () => {
        mocks.routes["DELETE /users/9"] = () => ({ data: {}, status: 200 });
        const s = makeStore();
        expect((await s.deleteUser("9")).success).toBe(true);
        expect(mocks.api.delete).toHaveBeenCalledWith("/users/9");

        mocks.routes["DELETE /users/9"] = () => { throw axError(404, { message: "Missing" }); };
        const result = await s.deleteUser("9");
        expect(result.success).toBe(false);
        if (!result.success) expect(result.error).toBe("Missing");
    });
});
