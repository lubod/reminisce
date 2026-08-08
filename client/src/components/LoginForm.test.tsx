import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { StoreContext } from "../stores/RootStore";
import { LoginForm } from "./LoginForm";

type AuthLike = {
    initialized: boolean;
    needsSetup: boolean;
    checkSetupStatus: ReturnType<typeof vi.fn>;
    setupAdmin: ReturnType<typeof vi.fn>;
    login: ReturnType<typeof vi.fn>;
};

function makeAuth(over: Partial<AuthLike> = {}): AuthLike {
    return {
        initialized: true,
        needsSetup: false,
        checkSetupStatus: vi.fn(() => Promise.resolve()),
        setupAdmin: vi.fn(() => Promise.resolve({ success: true })),
        login: vi.fn(() => Promise.resolve({ success: true })),
        ...over,
    };
}

function renderLogin(auth: AuthLike) {
    return render(
        <StoreContext.Provider value={{ authStore: auth } as never}>
            <LoginForm />
        </StoreContext.Provider>,
    );
}

// checkSetupStatus().finally() flips isLoading back to false on the following macrotask.
const flush = () => new Promise<void>(r => setTimeout(r, 0));

let href = "";
let search = "";
const originalLocation = window.location;

describe("LoginForm", () => {
    beforeEach(() => {
        href = "";
        search = "";
        // jsdom throws on navigation; use a settable stub so href assignment is observable.
        Object.defineProperty(window, "location", {
            configurable: true,
            value: {
                get href() { return href; },
                set href(v: string) { href = v; },
                get search() { return search; },
                set search(v: string) { search = v; },
                get pathname() { return "/"; },
            },
        });
    });
    afterEach(() => {
        Object.defineProperty(window, "location", { configurable: true, value: originalLocation });
    });

    it("shows a loader until auth is initialized", () => {
        const auth = makeAuth({ initialized: false });
        renderLogin(auth);
        expect(screen.queryByText("Sign in to your account")).toBeNull();
    });

    it("checks setup status on mount", () => {
        const auth = makeAuth();
        renderLogin(auth);
        expect(auth.checkSetupStatus).toHaveBeenCalledTimes(1);
    });

    it("renders the first-run setup form when needsSetup", async () => {
        const auth = makeAuth({ needsSetup: true });
        renderLogin(auth);
        await flush();
        expect(screen.getByText("Create your administrator account to get started.")).toBeTruthy();
        expect(screen.getByText("Create Admin Account")).toBeTruthy();
    });

    it("rejects mismatched setup passwords", async () => {
        const auth = makeAuth({ needsSetup: true });
        renderLogin(auth);
        await flush();
        const user = userEvent.setup();

        await user.type(screen.getByLabelText("Username"), "admin");
        await user.type(screen.getByLabelText("Password"), "password1");
        await user.type(screen.getByLabelText("Confirm Password"), "different");
        await user.click(screen.getByText("Create Admin Account"));

        expect(screen.getByText("Passwords do not match")).toBeTruthy();
        expect(auth.setupAdmin).not.toHaveBeenCalled();
    });

    it("rejects a setup password shorter than 8 characters", async () => {
        const auth = makeAuth({ needsSetup: true });
        renderLogin(auth);
        await flush();
        const user = userEvent.setup();

        await user.type(screen.getByLabelText("Username"), "admin");
        await user.type(screen.getByLabelText("Password"), "short1");
        await user.type(screen.getByLabelText("Confirm Password"), "short1");
        await user.click(screen.getByText("Create Admin Account"));

        expect(screen.getByText("Password must be at least 8 characters")).toBeTruthy();
        expect(auth.setupAdmin).not.toHaveBeenCalled();
    });

    it("surfaces a setup error without navigating when the login fails", async () => {
        const auth = makeAuth({
            needsSetup: true,
            setupAdmin: vi.fn(() => Promise.resolve({ success: true })),
            login: vi.fn(() => Promise.resolve({ success: false, error: "Login failed" })),
        });
        renderLogin(auth);
        await flush();
        const user = userEvent.setup();

        await user.type(screen.getByLabelText("Username"), "admin");
        await user.type(screen.getByLabelText("Password"), "password1");
        await user.type(screen.getByLabelText("Confirm Password"), "password1");
        await user.click(screen.getByText("Create Admin Account"));

        expect(auth.setupAdmin).toHaveBeenCalledWith("admin", "password1");
        expect(auth.login).toHaveBeenCalledWith("admin", "password1");
        expect(href).toBe("");
    });

    it("navigates to the home page after successful setup + login", async () => {
        const auth = makeAuth({ needsSetup: true });
        renderLogin(auth);
        await flush();
        const user = userEvent.setup();

        await user.type(screen.getByLabelText("Username"), "admin");
        await user.type(screen.getByLabelText("Password"), "password1");
        await user.type(screen.getByLabelText("Confirm Password"), "password1");
        await user.click(screen.getByText("Create Admin Account"));

        expect(href).toBe("/");
    });

    it("sign-in error from the server query param is shown", () => {
        search = "?error=1";
        renderLogin(makeAuth());
        expect(screen.getByText("Invalid username or password")).toBeTruthy();
    });

    it("renders the normal login form when not in setup", () => {
        renderLogin(makeAuth());
        expect(screen.getByText("Sign in to your account")).toBeTruthy();
        expect(screen.getByText("Sign In")).toBeTruthy();
    });
});
