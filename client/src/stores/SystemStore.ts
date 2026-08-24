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

export interface WorkerStats {
    id: string;
    name: string;
    count: number;
    mean_ms: number | null;
    p50_ms: number | null;
    p90_ms: number | null;
    p95_ms: number | null;
    p99_ms: number | null;
}

export interface HttpStats {
    total: number;
    per_second: number;
    status: { http_2xx: number; http_3xx: number; http_4xx: number; http_5xx: number };
    duration_ms: WorkerStats;
}

export interface PipelineResponse {
    workers: WorkerStats[];
    http: HttpStats;
    db_query_ms: WorkerStats;
}

export interface SeriesPoint {
    t: number;
    v: number;
}

export interface Series {
    name: string;
    unit: string;
    points: SeriesPoint[];
}

export interface SeriesResponse {
    range: string;
    series: Series[];
}

export type SeriesRange = "1d" | "30d" | "90d";

export interface AiModelInfo {
    id: string;
    name: string;
    model_id: string;
    task: string;
    loaded: boolean;
    status: string;
    dim?: number;
}

export interface AiModelsResponse {
    status: string;
    device: string;
    models: AiModelInfo[];
}


export class SystemStore {
    root: RootStore;

    logs: LogLine[] = [];
    logSource = "ring";
    errors: LogLine[] = [];
    errorCounts: ErrorCounts = { error: 0, warn: 0, panic: 0 };
    alerts: SystemAlert[] = [];
    gpu: GpuResponse | null = null;
    aiModels: AiModelsResponse | null = null;
    pipeline: PipelineResponse | null = null;
    series: Series[] = [];
    seriesRange: SeriesRange = "1d";

    isLoading = false;
    lastError: string | null = null;

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

    async loadGpu(): Promise<void> {
        try {
            const resp = await axios.get<GpuResponse>("/admin/gpu");
            this.gpu = resp.data;
            this.lastError = null;
        } catch (e) {
            this.recordError("loadGpu", e);
        }
    }

    
    async loadAiModels(): Promise<void> {
        try {
            const resp = await axios.get<AiModelsResponse>("/admin/ai-models");
            this.aiModels = resp.data;
            this.lastError = null;
        } catch (e) {
            this.recordError("loadAiModels", e);
        }
    }

    async loadPipeline(): Promise<void> {
        try {
            const resp = await axios.get<PipelineResponse>("/admin/pipeline");
            this.pipeline = resp.data;
            this.lastError = null;
        } catch (e) {
            this.recordError("loadPipeline", e);
        }
    }

    async loadSeries(): Promise<void> {
        try {
            const resp = await axios.get<SeriesResponse>("/admin/series", {
                params: { range: this.seriesRange },
            });
            this.series = resp.data.series;
            this.lastError = null;
        } catch (e) {
            this.recordError("loadSeries", e);
        }
    }

    setRange(range: SeriesRange): void {
        if (range === this.seriesRange) return;
        this.seriesRange = range;
        void this.loadSeries();
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