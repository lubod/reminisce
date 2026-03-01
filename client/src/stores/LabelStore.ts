import { makeAutoObservable, runInAction } from "mobx";
import axios from "../api/axiosConfig";
import { RootStore } from "./RootStore";

export interface Label {
    id: number;
    name: string;
    color: string;
    created_at: string;
}

export interface LabelsResponse {
    labels: Label[];
}

export interface CreateLabelRequest {
    name: string;
    color?: string;
}

export class LabelStore {
    rootStore: RootStore;
    labels: Label[] = [];
    isLoading: boolean = false;

    constructor(rootStore: RootStore) {
        makeAutoObservable(this);
        this.rootStore = rootStore;
    }

    fetchLabels = async () => {
        this.isLoading = true;
        try {
            const response = await axios.get<LabelsResponse>("/labels");
            runInAction(() => {
                this.labels = response.data.labels;
            });
        } catch (error) {
            console.error("Failed to fetch labels", error);
            this.rootStore.uiStore.setError("Failed to fetch labels");
        } finally {
            runInAction(() => {
                this.isLoading = false;
            });
        }
    };

    createLabel = async (name: string, color: string = "#3B82F6") => {
        try {
            const response = await axios.post<Label>("/labels", { name, color });
            runInAction(() => {
                this.labels.push(response.data);
            });
            return response.data;
        } catch (error) {
            console.error("Failed to create label", error);
            this.rootStore.uiStore.setError("Failed to create label");
            throw error;
        }
    };

    deleteLabel = async (labelId: number) => {
        try {
            await axios.delete(`/labels/${labelId}`);
            runInAction(() => {
                this.labels = this.labels.filter(l => l.id !== labelId);
            });
        } catch (error) {
            console.error("Failed to delete label", error);
            this.rootStore.uiStore.setError("Failed to delete label");
            throw error;
        }
    };

    getImageLabels = async (hash: string): Promise<Label[]> => {
        try {
            const response = await axios.get<LabelsResponse>(`/images/${hash}/labels`);
            return response.data.labels;
        } catch (error) {
            console.error("Failed to get image labels", error);
            return [];
        }
    };

    addImageLabel = async (hash: string, labelId: number) => {
        try {
            await axios.post(`/images/${hash}/labels`, { label_id: labelId });
        } catch (error) {
            console.error("Failed to add image label", error);
            this.rootStore.uiStore.setError("Failed to add label");
            throw error;
        }
    };

    removeImageLabel = async (hash: string, labelId: number) => {
        try {
            await axios.delete(`/images/${hash}/labels/${labelId}`);
        } catch (error) {
            console.error("Failed to remove image label", error);
            this.rootStore.uiStore.setError("Failed to remove label");
            throw error;
        }
    };

    getVideoLabels = async (hash: string): Promise<Label[]> => {
        try {
            const response = await axios.get<LabelsResponse>(`/videos/${hash}/labels`);
            return response.data.labels;
        } catch (error) {
            console.error("Failed to get video labels", error);
            return [];
        }
    };

    addVideoLabel = async (hash: string, labelId: number) => {
        try {
            await axios.post(`/videos/${hash}/labels`, { label_id: labelId });
        } catch (error) {
            console.error("Failed to add video label", error);
            this.rootStore.uiStore.setError("Failed to add label");
            throw error;
        }
    };

    removeVideoLabel = async (hash: string, labelId: number) => {
        try {
            await axios.delete(`/videos/${hash}/labels/${labelId}`);
        } catch (error) {
            console.error("Failed to remove video label", error);
            this.rootStore.uiStore.setError("Failed to remove label");
            throw error;
        }
    };
}
