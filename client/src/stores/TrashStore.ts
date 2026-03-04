import { makeAutoObservable, runInAction } from "mobx";
import { RootStore } from "./RootStore";
import api from "../api/axiosConfig";

export interface TrashItem {
    hash: string;
    name: string;
    created_at: string;
    ext: string;
    type: string;
    deviceid: string | null;
    deleted_at: string;
    media_type: "image" | "video";
}

export class TrashStore {
    rootStore: RootStore;
    items: TrashItem[] = [];
    isLoading: boolean = false;
    error: string | null = null;

    constructor(rootStore: RootStore) {
        makeAutoObservable(this);
        this.rootStore = rootStore;
    }

    getThumbnailUrl = (item: TrashItem): string => {
        const base = `/api/thumbnail/${item.hash}`;
        const token = this.rootStore.authStore.token;
        if (!token) return base;
        return `${base}?token=${token}`;
    };

    fetchTrash = async () => {
        this.isLoading = true;
        this.error = null;
        try {
            const response = await api.get<TrashItem[]>("/trash");
            runInAction(() => {
                this.items = response.data;
                this.isLoading = false;
            });
        } catch {
            runInAction(() => {
                this.error = "Failed to load trash";
                this.isLoading = false;
            });
        }
    };

    restoreItem = async (hash: string, media_type: "image" | "video") => {
        try {
            await api.post(`/${media_type}/${hash}/restore`);
            runInAction(() => {
                this.items = this.items.filter((item) => item.hash !== hash);
            });
        } catch {
            runInAction(() => {
                this.error = "Failed to restore item";
            });
        }
    };
}
