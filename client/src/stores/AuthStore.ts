import { makeAutoObservable } from "mobx";
import { RootStore } from "./RootStore";
import api from "../api/axiosConfig";
import axios from "axios";

export interface User {
    id: string;
    username: string;
    role: string;
}

export class AuthStore {
    token: string | null = localStorage.getItem("token");
    user: User | null = JSON.parse(localStorage.getItem("user") || "null");
    isAuthenticated: boolean = !!this.token;
    rootStore: RootStore;

    constructor(rootStore: RootStore) {
        makeAutoObservable(this);
        this.rootStore = rootStore;
    }

    login = async (username: string, password: string) => {
        try {
            // Call the new user login endpoint
            const response = await api.post("/auth/user-login", {
                username,
                password,
            });
            const token = response.data.access_token;
            const user = response.data.user;
            this.setToken(token);
            this.setUser(user);
            return { success: true };
        } catch (error: unknown) {
            console.error("Login failed", error);
            let message = "An unknown error occurred";
            if (axios.isAxiosError(error) && error.response) {
                message = error.response.data?.message || `Error: ${error.message}`;
            } else if (error instanceof Error) {
                message = error.message;
            }
            return { success: false, error: message };
        }
    };

    register = async (username: string, email: string, password: string) => {
        try {
            await api.post("/auth/register", {
                username,
                email,
                password,
            });
            return { success: true };
        } catch (error: unknown) {
            console.error("Registration failed", error);
            let message = "An unknown error occurred";
            if (axios.isAxiosError(error) && error.response) {
                message = error.response.data?.message || `Error: ${error.message}`;
            } else if (error instanceof Error) {
                message = error.message;
            }
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
        if (token) {
            localStorage.setItem("token", token);
        } else {
            localStorage.removeItem("token");
        }
    };

    setUser = (user: User | null) => {
        this.user = user;
        if (user) {
            localStorage.setItem("user", JSON.stringify(user));
        } else {
            localStorage.removeItem("user");
        }
    };
}
