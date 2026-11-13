import { makeAutoObservable, runInAction } from "mobx";
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
    rootStore: RootStore;

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

    lastError: string | null = null;

    constructor(rootStore: RootStore) {
        this.rootStore = rootStore;
        makeAutoObservable(this);
    }

    private async fetchInto<T>(
        context: string,
        url: string,
        apply: (data: T) => void,
        params?: Record<string, string | number>
    ): Promise<void> {
        try {
            const resp = params
                ? await axios.get<T>(url, { params })
                : await axios.get<T>(url);
            runInAction(() => {
                apply(resp.data);
                this.lastError = null;
            });
        } catch (e) {
            this.recordError(context, e);
        }
    }

    async loadLogs(level = "info", limit = 300): Promise<void> {
        await this.fetchInto<LogsResponse>(
            "loadLogs",
            "/admin/logs",
            (data) => {
                this.logs = data.entries;
                this.logSource = data.source;
            },
            { level, limit }
        );
    }

    async loadErrors(): Promise<void> {
        await this.fetchInto<ErrorsResponse>("loadErrors", "/admin/errors", (data) => {
            this.errors = data.entries;
            this.errorCounts = data.count_5m;
        });
    }

    async loadAlerts(): Promise<void> {
        await this.fetchInto<AlertsResponse>("loadAlerts", "/admin/alerts", (data) => {
            this.alerts = data.alerts;
        });
    }

    async loadGpu(): Promise<void> {
        await this.fetchInto<GpuResponse>("loadGpu", "/admin/gpu", (data) => {
            this.gpu = data;
        });
    }

    async loadAiModels(): Promise<void> {
        await this.fetchInto<AiModelsResponse>("loadAiModels", "/admin/ai-models", (data) => {
            this.aiModels = data;
        });
    }

    async loadPipeline(): Promise<void> {
        await this.fetchInto<PipelineResponse>("loadPipeline", "/admin/pipeline", (data) => {
            this.pipeline = data;
        });
    }

    async loadSeries(): Promise<void> {
        await this.fetchInto<SeriesResponse>(
            "loadSeries",
            "/admin/series",
            (data) => {
                this.series = data.series;
            },
            { range: this.seriesRange }
        );
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
        const msg = `${context}: ${detail}`;
        runInAction(() => {
            this.lastError = msg;
        });
        logger.warn(msg);
    }
}