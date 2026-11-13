import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { UserManagement } from "./UserManagement";
import { StoreContext } from "../stores/RootStore";

const usersFixture = [
    { id: "admin-1", username: "admin", role: "admin", is_active: true, created_at: "2024-01-01T00:00:00Z" },
    { id: "user-2", username: "alice", role: "user", is_active: true, created_at: "2024-01-02T00:00:00Z" },
];

function makeStores() {
    const setSuccess = vi.fn();
    const setError = vi.fn();
    const listUsers = vi.fn(async () => usersFixture);
    const updateUser = vi.fn(async () => ({ success: false, error: "Server rejected update" }));
    const deleteUser = vi.fn(async () => ({ success: false, error: "Cannot remove user" }));

    const rootStore = {
        authStore: {
            user: { id: "admin-1", username: "admin", role: "admin" },
            listUsers,
            createUser: vi.fn(async () => ({ success: true })),
            updateUser,
            deleteUser,
        },
        uiStore: {
            setSuccess,
            setError,
        },
    };
    return { rootStore, setSuccess, setError, updateUser, deleteUser };
}

describe("UserManagement", () => {
    beforeEach(() => {
        vi.restoreAllMocks();
    });

    it("shows error toast instead of success toast when updateUser returns success: false", async () => {
        const { rootStore, setSuccess, setError, updateUser } = makeStores();
        render(
            <StoreContext.Provider value={rootStore as never}>
                <UserManagement />
            </StoreContext.Provider>
        );

        await screen.findByText("alice");
        const user = userEvent.setup();
        await user.click(screen.getAllByTitle("Disable user")[1]);

        await waitFor(() => expect(updateUser).toHaveBeenCalledWith("user-2", { is_active: false }));
        expect(setError).toHaveBeenCalledWith("Server rejected update");
        expect(setSuccess).not.toHaveBeenCalled();
    });

    it("shows error toast instead of success toast when deleteUser returns success: false", async () => {
        vi.spyOn(window, "confirm").mockReturnValue(true);
        const { rootStore, setSuccess, setError, deleteUser } = makeStores();
        render(
            <StoreContext.Provider value={rootStore as never}>
                <UserManagement />
            </StoreContext.Provider>
        );

        await screen.findByText("alice");
        const user = userEvent.setup();
        await user.click(screen.getAllByTitle("Delete user")[1]);

        await waitFor(() => expect(deleteUser).toHaveBeenCalledWith("user-2"));
        expect(setError).toHaveBeenCalledWith("Cannot remove user");
        expect(setSuccess).not.toHaveBeenCalled();
    });
});
