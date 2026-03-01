import { makeAutoObservable } from "mobx";
import { RootStore } from "./RootStore";

export class UIStore {
    rootStore: RootStore;
    isLoading: boolean = false;
    error: string | null = null;
    success: string | null = null;
    isFullscreen: boolean = false;

    constructor(rootStore: RootStore) {
        makeAutoObservable(this);
        this.rootStore = rootStore;
    }

    setLoading(loading: boolean) {
        this.isLoading = loading;
    }

    setError(error: string | null) {
        this.error = error;
        if (error) {
            this.success = null; // Clear success when showing error
        }
    }

    setSuccess(success: string | null) {
        this.success = success;
        if (success) {
            this.error = null; // Clear error when showing success
        }
    }

    setIsFullscreen(value: boolean) {
        this.isFullscreen = value;
    }
}
