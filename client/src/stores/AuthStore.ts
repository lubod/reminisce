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
    token: string | null = localStorage.getItem("token");
    user: User | null = JSON.parse(localStorage.getItem("user") || "null");
    isAuthenticated: boolean = !!this.token;
    needsSetup: boolean = false;
    rootStore: RootStore;

    constructor(rootStore: RootStore) {
        makeAutoObservable(this);
        this.rootStore = rootStore;
    }

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

    logout = () => {
        this.setToken(null);
        this.setUser(null);
    };

    setToken = (token: string | null) => {
        this.token = token;
        this.isAuthenticated = !!token;
        if (token) localStorage.setItem("token", token);
        else localStorage.removeItem("token");
    };

    setUser = (user: User | null) => {
        this.user = user;
        if (user) localStorage.setItem("user", JSON.stringify(user));
        else localStorage.removeItem("user");
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
