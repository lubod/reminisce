import { makeAutoObservable, runInAction } from "mobx";
import type { RootStore } from "./RootStore";

const ERROR_TOAST_MS = 6000;
const SUCCESS_TOAST_MS = 3500;

export class UIStore {
    rootStore: RootStore;
    isLoading: boolean = false;
    error: string | null = null;
    success: string | null = null;
    isFullscreen: boolean = false;

    private errorTimer: ReturnType<typeof setTimeout> | null = null;
    private successTimer: ReturnType<typeof setTimeout> | null = null;

    constructor(rootStore: RootStore) {
        makeAutoObservable(this);
        this.rootStore = rootStore;
    }

    setLoading(loading: boolean) {
        this.isLoading = loading;
    }

    setError(error: string | null) {
        this.error = error;
        if (this.errorTimer) {
            clearTimeout(this.errorTimer);
            this.errorTimer = null;
        }
        if (error) {
            this.success = null; // Clear success when showing error
            this.errorTimer = setTimeout(
                () => runInAction(() => { this.error = null; this.errorTimer = null; }),
                ERROR_TOAST_MS,
            );
        }
    }

    setSuccess(success: string | null) {
        this.success = success;
        if (this.successTimer) {
            clearTimeout(this.successTimer);
            this.successTimer = null;
        }
        if (success) {
            this.error = null; // Clear error when showing success
            this.successTimer = setTimeout(
                () => runInAction(() => { this.success = null; this.successTimer = null; }),
                SUCCESS_TOAST_MS,
            );
        }
    }

    dismissError() {
        this.setError(null);
    }

    dismissSuccess() {
        this.setSuccess(null);
    }

    setIsFullscreen(value: boolean) {
        this.isFullscreen = value;
    }
}
