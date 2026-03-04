import { makeAutoObservable, runInAction } from "mobx";
import { RootStore } from "./RootStore";
import api from "../api/axiosConfig";

export interface DuplicateImage {
    hash: string;
    deviceid: string;
    name: string;
    created_at: string;
    thumbnail_url: string;
    aesthetic_score: number | null;
    sharpness_score: number | null;
    width: number | null;
    height: number | null;
    file_size_bytes: number | null;
}

export interface DuplicateGroup {
    similarity: number;
    images: DuplicateImage[];
}

export interface DuplicatesResponse {
    groups: DuplicateGroup[];
    total_groups: number;
}

export class DuplicatesStore {
    rootStore: RootStore;
    groups: DuplicateGroup[] = [];
    isLoading: boolean = false;
    threshold: number = 0.95;
    error: string | null = null;

    constructor(rootStore: RootStore) {
        makeAutoObservable(this);
        this.rootStore = rootStore;
    }

    setThreshold(value: number) {
        this.threshold = Math.min(1.0, Math.max(0.8, value));
    }

    fetchDuplicates = async () => {
        this.isLoading = true;
        this.error = null;
        try {
            const response = await api.get<DuplicatesResponse>("/duplicates", {
                params: { threshold: this.threshold },
            });
            runInAction(() => {
                this.groups = response.data.groups;
                this.isLoading = false;
            });
        } catch (err: unknown) {
            runInAction(() => {
                this.error = "Failed to load duplicates";
                this.isLoading = false;
            });
        }
    };

    deleteImage = async (hash: string) => {
        try {
            await api.post(`/image/${hash}/delete`);
            runInAction(() => {
                // Remove all images with this hash from all groups (delete is by hash)
                this.groups = this.groups
                    .map((group) => ({
                        ...group,
                        images: group.images.filter((img) => img.hash !== hash),
                    }))
                    .filter((group) => group.images.length >= 2);
            });
        } catch (err: unknown) {
            runInAction(() => {
                this.error = "Failed to delete image";
            });
        }
    };
}
