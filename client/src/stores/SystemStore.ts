import { makeAutoObservable } from "mobx";
import axios from "../api/axiosConfig";
import { isAxiosError } from "axios";
import { logger } from "../utils/logger";
import type { RootStore } from "./RootStore";

export interface LogLine {
    timestamp: number;
    level: string;
    target: string;
    message: string;
    fields: Record<string, unknown>;
}

export interface LogsResponse {
    entries: LogLine[];
    source: string;
}

export interface ErrorCounts {
    error: number;
    warn: number;
    panic: number;
}

export interface ErrorsResponse {
    entries: LogLine[];
    count_5m: ErrorCounts;
}

export interface SystemAlert {
    id: string;
    severity: "ok" | "warning" | "critical";
    status: "ok" | "firing";
    message: string;
    detail: string;
    value: string;
}

export interface AlertsResponse {
    alerts: SystemAlert[];
}

export interface SystemStats {
    cpu_usage_percent: number;
    memory_usage_percent: number;
    memory_used_gb: number;
    memory_total_gb: number;
    disk_usage_percent: number;
    disk_available_gb: number;
    uptime_seconds: number;
    gpu_usage_percent: number | null;
    gpu_memory_used_mb: number | null;
    gpu_memory_total_mb: number | null;
    gpu_temp_celsius: number | null;
    cpu_temp_celsius: number | null;
}

export interface GpuCard {
    gpu: string;
    utilization_percent: number | null;
    memory_usage_percent: number | null;
    memory_used_bytes: number | null;
    memory_total_bytes: number | null;
    temperature_celsius: number | null;
}

export interface GpuResponse {
    available: boolean;
    cards: GpuCard[];
}

export interface BackupStatus {
    is_healthy: boolean;
    health_status: string;
    active_peers: number;
    ok_files: number;
    degraded_files: number;
    missing_files: number;
    pending_images: number;
    pending_videos: number;
    db_backups_count: number;
    db_backups_latest_at: string | null;
}

export interface PoolStats {
    main_pool: {
        size: number;
        available: number;
        max_size: number;
        utilization_percent: number;
    };
}

export class SystemStore {
    root: RootStore;

    logs: LogLine[] = [];
    logSource = "ring";
    errors: LogLine[] = [];
    errorCounts: ErrorCounts = { error: 0, warn: 0, panic: 0 };
    alerts: SystemAlert[] = [];
    system: SystemStats | null = null;
    pool: PoolStats | null = null;
    backup: BackupStatus | null = null;
    gpu: GpuResponse | null = null;

    isLoading = false;
    lastError: string | null = null;

    private refreshPromise: Promise<void> | null = null;
    private timer: ReturnType<typeof setInterval> | null = null;

    constructor(root: RootStore) {
        this.root = root;
        makeAutoObservable(this);
    }

    async loadLogs(level = "info", limit = 300): Promise<void> {
        try {
            const resp = await axios.get<LogsResponse>("/admin/logs", {
                params: { level, limit },
            });
            this.logs = resp.data.entries;
            this.logSource = resp.data.source;
            this.lastError = null;
        } catch (e) {
            this.recordError("loadLogs", e);
        }
    }

    async loadErrors(): Promise<void> {
        try {
            const resp = await axios.get<ErrorsResponse>("/admin/errors");
            this.errors = resp.data.entries;
            this.errorCounts = resp.data.count_5m;
            this.lastError = null;
        } catch (e) {
            this.recordError("loadErrors", e);
        }
    }

    async loadAlerts(): Promise<void> {
        try {
            const resp = await axios.get<AlertsResponse>("/admin/alerts");
            this.alerts = resp.data.alerts;
            this.lastError = null;
        } catch (e) {
            this.recordError("loadAlerts", e);
        }
    }

    async loadSystem(): Promise<void> {
        try {
            const resp = await axios.get<SystemStats>("/system-stats");
            this.system = resp.data;
            this.lastError = null;
        } catch (e) {
            this.recordError("loadSystem", e);
        }
    }

    async loadPool(): Promise<void> {
        try {
            const resp = await axios.get<PoolStats>("/pool-stats");
            this.pool = resp.data;
            this.lastError = null;
        } catch (e) {
            this.recordError("loadPool", e);
        }
    }

    async loadBackup(): Promise<void> {
        try {
            const resp = await axios.get<BackupStatus>("/p2p/backup/status");
            this.backup = resp.data;
            this.lastError = null;
        } catch (e) {
            this.recordError("loadBackup", e);
        }
    }

    async loadGpu(): Promise<void> {
        try {
            const resp = await axios.get<GpuResponse>("/admin/gpu");
            this.gpu = resp.data;
            this.lastError = null;
        } catch (e) {
            this.recordError("loadGpu", e);
        }
    }

    /** Refreshes everything exactly once (single-flight). */
    async refreshAll(): Promise<void> {
        if (this.refreshPromise) {
            return this.refreshPromise;
        }
        this.isLoading = true;
        this.refreshPromise = Promise.allSettled([
            this.loadLogs(),
            this.loadErrors(),
            this.loadAlerts(),
            this.loadSystem(),
            this.loadPool(),
            this.loadBackup(),
            this.loadGpu(),
        ])
            .then(() => {
                this.isLoading = false;
            })
            .finally(() => {
                this.refreshPromise = null;
            });
        return this.refreshPromise;
    }

    /** Begin auto-refresh. Returns a cleanup function. */
    startAutoRefresh(intervalMs = 5000): () => void {
        void this.refreshAll();
        this.timer = setInterval(() => {
            void this.refreshAll();
        }, intervalMs);
        return () => {
            if (this.timer) {
                clearInterval(this.timer);
                this.timer = null;
            }
        };
    }

    private recordError(context: string, e: unknown): void {
        let detail: string;
        if (isAxiosError(e)) {
            detail = `HTTP ${e.response?.status ?? "?"}: ${e.message}`;
        } else if (e instanceof Error) {
            detail = e.message;
        } else {
            detail = String(e);
        }
        this.lastError = `${context}: ${detail}`;
        logger.warn(this.lastError);
    }
}