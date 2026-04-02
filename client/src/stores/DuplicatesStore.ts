import { makeAutoObservable, runInAction } from "mobx";
import { RootStore } from "./RootStore";
import api from "../api/axiosConfig";

export interface DuplicateImage {
    hash: string;
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

interface DuplicatesResponse {
    groups: DuplicateGroup[];
    total_groups: number;
    page: number;
    limit: number;
}

export interface WorkerStatus {
    running: boolean;
    checked_images: number;
    total_images: number;
    total_pairs: number;
    last_completed_at: string | null;
}

const PAGE_SIZE = 20;

export class DuplicatesStore {
    rootStore: RootStore;
    groups: DuplicateGroup[] = [];
    totalGroups: number = 0;
    page: number = 1;
    isLoading: boolean = false;
    isLoadingMore: boolean = false;
    isTriggeringScan: boolean = false;
    threshold: number = 0.95;
    error: string | null = null;

    workerStatus: WorkerStatus | null = null;
    private statusPollTimer: ReturnType<typeof setInterval> | null = null;

    constructor(rootStore: RootStore) {
        makeAutoObservable(this);
        this.rootStore = rootStore;
    }

    get hasMore(): boolean {
        return this.groups.length < this.totalGroups;
    }

    setThreshold(value: number) {
        this.threshold = Math.min(1.0, Math.max(0.8, value));
    }

    fetchDuplicates = async () => {
        this.isLoading = true;
        this.error = null;
        this.page = 1;
        try {
            const response = await api.get<DuplicatesResponse>("/duplicates", {
                params: { threshold: this.threshold, page: 1, limit: PAGE_SIZE },
            });
            runInAction(() => {
                this.groups = response.data.groups;
                this.totalGroups = response.data.total_groups;
                this.page = 1;
                this.isLoading = false;
            });
        } catch {
            runInAction(() => {
                this.error = "Failed to load duplicates";
                this.isLoading = false;
            });
        }
    };

    loadNextPage = async () => {
        if (this.isLoadingMore || !this.hasMore) return;
        this.isLoadingMore = true;
        const nextPage = this.page + 1;
        try {
            const response = await api.get<DuplicatesResponse>("/duplicates", {
                params: { threshold: this.threshold, page: nextPage, limit: PAGE_SIZE },
            });
            runInAction(() => {
                this.groups = [...this.groups, ...response.data.groups];
                this.totalGroups = response.data.total_groups;
                this.page = nextPage;
                this.isLoadingMore = false;
            });
        } catch {
            runInAction(() => {
                this.error = "Failed to load more duplicates";
                this.isLoadingMore = false;
            });
        }
    };

    fetchWorkerStatus = async () => {
        try {
            const response = await api.get<WorkerStatus>("/duplicates/status");
            runInAction(() => {
                this.workerStatus = response.data;
            });
        } catch {
            // silently ignore
        }
    };

    startStatusPolling() {
        this.fetchWorkerStatus();
        if (this.statusPollTimer) clearInterval(this.statusPollTimer);
        this.statusPollTimer = setInterval(() => {
            this.fetchWorkerStatus();
        }, 5000);
    }

    stopStatusPolling() {
        if (this.statusPollTimer) {
            clearInterval(this.statusPollTimer);
            this.statusPollTimer = null;
        }
    }

    triggerScan = async () => {
        this.isTriggeringScan = true;
        this.error = null;
        try {
            await api.post("/duplicates/scan");
            // Refresh status immediately after triggering
            await this.fetchWorkerStatus();
        } catch {
            runInAction(() => {
                this.error = "Failed to trigger scan";
            });
        } finally {
            runInAction(() => {
                this.isTriggeringScan = false;
            });
        }
    };

    deleteImage = async (hash: string) => {
        try {
            await api.post(`/image/${hash}/delete`);
            runInAction(() => {
                this.groups = this.groups
                    .map((group) => ({
                        ...group,
                        images: group.images.filter((img) => img.hash !== hash),
                    }))
                    .filter((group) => group.images.length >= 2);
                this.totalGroups = this.groups.length;
            });
        } catch {
            runInAction(() => {
                this.error = "Failed to delete image";
            });
        }
    };
}
