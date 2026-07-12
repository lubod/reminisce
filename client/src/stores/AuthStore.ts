import { makeAutoObservable } from "mobx";
import { RootStore } from "./RootStore";
import api from "../api/axiosConfig";
import axios from "axios";

export interface User {
    id: string;
    username: string;
    role: string;
}

export interface ManagedUser {
    id: string;
    username: string;
    email: string;
    role: string;
    is_active: boolean;
    created_at: string;
    last_login_at: string | null;
}

export class AuthStore {
    token: string | null = null; // Memory-only token to prevent XSS localStorage theft (H2)
    imageToken: string | null = null; // Scoped memory-only token for media/images (H2)
    user: User | null = null;
    isAuthenticated: boolean = false;
    needsSetup: boolean = false;
    initialized: boolean = false;
    rootStore: RootStore;

    constructor(rootStore: RootStore) {
        makeAutoObservable(this);
        this.rootStore = rootStore;
    }

    /** Called once on app startup. Checks setup status and validates any cached token. */
    initialize = async () => {
        try {
            const res = await api.get("/auth/setup-status");
            this.needsSetup = res.data.needs_setup;
            if (this.needsSetup) {
                this.setUser(null);
                this.setToken(null);
                this.setImageToken(null);
            } else {
                // Fetch currently authenticated user session and token
                try {
                    const meRes = await api.get("/auth/me");
                    this.setUser(meRes.data);
                    this.setToken(meRes.data.access_token);
                    this.setImageToken(meRes.data.image_token);
                } catch (err) {
                    this.setUser(null);
                    this.setToken(null);
                    this.setImageToken(null);
                }
            }
        } catch {
            this.needsSetup = false;
        } finally {
            this.initialized = true;
        }
    };

    checkSetupStatus = async () => {
        try {
            const res = await api.get("/auth/setup-status");
            this.needsSetup = res.data.needs_setup;
        } catch {
            this.needsSetup = false;
        }
    };

    setupAdmin = async (username: string, password: string) => {
        try {
            await api.post("/auth/setup", { username, password });
            this.needsSetup = false;
            return { success: true };
        } catch (error: unknown) {
            let message = "Setup failed";
            if (axios.isAxiosError(error) && error.response)
                message = error.response.data?.message || message;
            return { success: false, error: message };
        }
    };

    login = async (username: string, password: string) => {
        try {
            const response = await api.post("/auth/user-login", { username, password });
            this.setToken(response.data.access_token);
            this.setImageToken(response.data.image_token);
            this.setUser(response.data.user);
            return { success: true };
        } catch (error: unknown) {
            let message = "An unknown error occurred";
            if (axios.isAxiosError(error) && error.response)
                message = error.response.data?.message || `Error: ${error.message}`;
            else if (error instanceof Error)
                message = error.message;
            return { success: false, error: message };
        }
    };

    logout = async () => {
        try {
            await api.post("/auth/logout");
        } catch (e) {
            console.error("Logout request failed", e);
        }
        this.setToken(null);
        this.setImageToken(null);
        this.setUser(null);
    };

    setToken = (token: string | null) => {
        this.token = token;
    };

    setImageToken = (token: string | null) => {
        this.imageToken = token;
    };

    setUser = (user: User | null) => {
        this.user = user;
        this.isAuthenticated = !!user;
    };

    // --- User management (admin only) ---

    listUsers = async (): Promise<ManagedUser[]> => {
        const res = await api.get("/users");
        return res.data;
    };

    createUser = async (username: string, password: string, role: string) => {
        try {
            await api.post("/users", { username, password, role });
            return { success: true };
        } catch (error: unknown) {
            let message = "Failed to create user";
            if (axios.isAxiosError(error) && error.response)
                message = error.response.data?.message || message;
            return { success: false, error: message };
        }
    };

    updateUser = async (id: string, updates: { role?: string; is_active?: boolean; password?: string }) => {
        try {
            await api.patch(`/users/${id}`, updates);
            return { success: true };
        } catch (error: unknown) {
            let message = "Failed to update user";
            if (axios.isAxiosError(error) && error.response)
                message = error.response.data?.message || message;
            return { success: false, error: message };
        }
    };

    deleteUser = async (id: string) => {
        try {
            await api.delete(`/users/${id}`);
            return { success: true };
        } catch (error: unknown) {
            let message = "Failed to delete user";
            if (axios.isAxiosError(error) && error.response)
                message = error.response.data?.message || message;
            return { success: false, error: message };
        }
    };
}
