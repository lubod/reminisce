import { useEffect } from "react";
import { observer } from "mobx-react-lite";
import { useStore } from "../../stores/RootStore";
import { LogViewer } from "./LogViewer";
import {
    Activity,
    AlertTriangle,
    CheckCircle2,
    Cpu,
    Database,
    HardDrive,
    ServerCrash,
    ShieldAlert,
    Users,
} from "lucide-react";

const severityColor = (severity: string) => {
    switch (severity) {
        case "critical":
            return "bg-red-900/60 border-red-700 text-red-200";
        case "warning":
            return "bg-amber-900/40 border-amber-700 text-amber-200";
        default:
            return "bg-emerald-900/40 border-emerald-700 text-emerald-200";
    }
};

const severityText = (severity: string) => {
    switch (severity) {
        case "critical":
            return "bg-red-600";
        case "warning":
            return "bg-amber-500";
        default:
            return "bg-emerald-500";
    }
};

function Card({ title, icon, children }: { title: string; icon: React.ReactNode; children: React.ReactNode }) {
    return (
        <div className="bg-gray-800 border border-gray-700 rounded-lg p-4">
            <div className="flex items-center gap-2 text-gray-300 text-sm font-semibold mb-3">
                {icon}
                {title}
            </div>
            {children}
        </div>
    );
}

function Metric({ label, value, sub }: { label: string; value: React.ReactNode; sub?: string }) {
    return (
        <div className="flex items-baseline justify-between gap-2">
            <span className="text-gray-400 text-xs">{label}</span>
            <span className="text-gray-100 font-mono text-sm">
                {value}
                {sub ? <span className="text-gray-500 text-xs ml-1">{sub}</span> : null}
            </span>
        </div>
    );
}

export const SystemPage = observer(() => {
    const { systemStore } = useStore();

    useEffect(() => {
        return systemStore.startAutoRefresh(5000);
    }, [systemStore]);

    const alerts = systemStore.alerts;
    const firing = alerts.filter((a) => a.status === "firing");
    const sys = systemStore.system;
    const backup = systemStore.backup;
    const pool = systemStore.pool;
    const gpu = systemStore.gpu;

    return (
        <div className="space-y-6">
            <div className="flex items-center justify-between">
                <h1 className="text-2xl font-bold text-gray-100 flex items-center gap-2">
                    <Activity className="w-6 h-6 text-blue-400" />
                    System
                </h1>
                {systemStore.isLoading ? (
                    <span className="text-xs text-gray-500">refreshing…</span>
                ) : null}
            </div>

            {systemStore.lastError ? (
                <div className="bg-red-900/40 border border-red-800 rounded-lg px-4 py-2 text-red-200 text-sm">
                    {systemStore.lastError}
                </div>
            ) : null}

            {/* Alerts */}
            <Card title={`Alerts (${firing.length}/${alerts.length})`} icon={<ShieldAlert className="w-4 h-4 text-amber-400" />}>
                {alerts.length === 0 ? (
                    <p className="text-gray-500 text-sm">No alert data yet.</p>
                ) : (
                    <div className="grid gap-1.5">
                        {alerts.map((a) => (
                            <div
                                key={a.id}
                                className={`flex items-center gap-3 rounded border px-3 py-1.5 text-sm ${severityColor(a.severity)}`}
                            >
                                <span className={`w-2 h-2 rounded-full shrink-0 ${severityText(a.severity)}`} />
                                <span className="font-medium">{a.message}</span>
                                <span className="text-xs opacity-80 truncate flex-1">{a.detail}</span>
                                {a.id === a.id && a.value ? (
                                    <span className="font-mono text-xs opacity-80">{a.value}</span>
                                ) : null}
                            </div>
                        ))}
                    </div>
                )}
            </Card>

            {/* Error counts */}
            <div className="grid grid-cols-3 gap-3">
                <div className="bg-red-950/50 border border-red-800 rounded-lg p-3 text-center">
                    <div className="text-2xl font-bold text-red-400">{systemStore.errorCounts.error}</div>
                    <div className="text-xs text-gray-400">errors / 5m</div>
                </div>
                <div className="bg-amber-950/50 border border-amber-800 rounded-lg p-3 text-center">
                    <div className="text-2xl font-bold text-amber-400">{systemStore.errorCounts.warn}</div>
                    <div className="text-xs text-gray-400">warnings / 5m</div>
                </div>
                <div className="bg-purple-950/50 border border-purple-800 rounded-lg p-3 text-center">
                    <div className="text-2xl font-bold text-purple-400">{systemStore.errorCounts.panic}</div>
                    <div className="text-xs text-gray-400">panics / 5m</div>
                </div>
            </div>

            {/* Data cards */}
            <div className="grid md:grid-cols-2 xl:grid-cols-3 gap-4">
                <Card title="Backup / P2P" icon={<Database className="w-4 h-4 text-blue-400" />}>
                    {backup ? (
                        <div className="space-y-1.5">
                            <Metric label="Health" value={backup.health_status} />
                            <Metric label="Active peers" value={backup.active_peers} />
                            <Metric label="OK files" value={backup.ok_files} />
                            <Metric
                                label="Degraded / missing"
                                value={`${backup.degraded_files} / ${backup.missing_files}`}
                                sub={backup.degraded_files + backup.missing_files > 0 ? "⚠" : ""}
                            />
                            <Metric label="Pending" value={`${backup.pending_images} img / ${backup.pending_videos} vid`} />
                            <Metric label="DB backups" value={`${backup.db_backups_count} · ${backup.db_backups_latest_at ?? "never"}`} />
                        </div>
                    ) : (
                        <p className="text-gray-500 text-sm">—</p>
                    )}
                </Card>

                <Card title="Database pool" icon={<ServerCrash className="w-4 h-4 text-blue-400" />}>
                    {pool ? (
                        <div className="space-y-1.5">
                            <Metric label="In use" value={`${pool.main_pool.size - pool.main_pool.available}/${pool.main_pool.max_size}`} />
                            <Metric label="Utilization" value={`${pool.main_pool.utilization_percent?.toFixed?.(0) ?? "?"}%`} />
                        </div>
                    ) : (
                        <p className="text-gray-500 text-sm">—</p>
                    )}
                </Card>

                <Card title="System load" icon={<Cpu className="w-4 h-4 text-blue-400" />}>
                    {sys ? (
                        <div className="space-y-1.5">
                            <Metric label="CPU" value={`${sys.cpu_usage_percent?.toFixed?.(0) ?? "?"}%`} />
                            <Metric label="Memory" value={`${sys.memory_used_gb?.toFixed?.(1) ?? "?"} GB`} sub={`/ ${sys.memory_total_gb?.toFixed?.(0) ?? "?"}`} />
                            <Metric label="Disk free" value={`${sys.disk_available_gb?.toFixed?.(1) ?? "?"} GB`} />
                            <Metric label="Temp" value={`${sys.cpu_temp_celsius != null ? `${sys.cpu_temp_celsius?.toFixed?.(0)}°C` : "—"}`} />
                        </div>
                    ) : (
                        <p className="text-gray-500 text-sm">—</p>
                    )}
                </Card>

                <Card title="GPU (AI)" icon={<HardDrive className="w-4 h-4 text-blue-400" />}>
                    {gpu && gpu.available ? (
                        gpu.cards.map((c) => (
                            <div key={c.gpu} className="space-y-1.5">
                                <Metric label={`GPU ${c.gpu}`} value={`${c.utilization_percent?.toFixed?.(0) ?? "?"}%`} />
                                <Metric
                                    label="VRAM"
                                    value={
                                        c.memory_used_bytes && c.memory_total_bytes
                                            ? `${(c.memory_used_bytes / 1024 ** 3).toFixed(1)} GB`
                                            : "—"
                                    }
                                    sub={c.memory_total_bytes ? `/ ${(c.memory_total_bytes / 1024 ** 3).toFixed(1)} GB` : ""}
                                />
                                <Metric label="Temp" value={c.temperature_celsius != null ? `${c.temperature_celsius.toFixed(0)}°C` : "—"} />
                            </div>
                        ))
                    ) : (
                        <p className="text-gray-500 text-sm">Unavailable (no GPU metrics)</p>
                    )}
                </Card>

                <Card title="Users / auth" icon={<Users className="w-4 h-4 text-blue-400" />}>
                    <div className="flex items-center gap-2 text-sm text-gray-300">
                        <CheckCircle2 className="w-4 h-4 text-emerald-400" />
                        <span>Session authenticated</span>
                    </div>
                </Card>

                <Card title="Uptime" icon={<AlertTriangle className="w-4 h-4 text-blue-400" />}>
                    {sys ? (
                        <div className="space-y-1.5">
                            <Metric label="Uptime" value={formatUptime(sys.uptime_seconds ?? 0)} />
                        </div>
                    ) : (
                        <p className="text-gray-500 text-sm">—</p>
                    )}
                </Card>
            </div>

            {/* Logs */}
            <Card title={`Logs (${systemStore.logSource})`} icon={<Activity className="w-4 h-4 text-blue-400" />}>
                <LogViewer />
            </Card>
        </div>
    );
});

function formatUptime(seconds: number): string {
    if (!seconds) return "—";
    const d = Math.floor(seconds / 86400);
    const h = Math.floor((seconds % 86400) / 3600);
    const m = Math.floor((seconds % 3600) / 60);
    if (d > 0) return `${d}d ${h}h`;
    if (h > 0) return `${h}h ${m}m`;
    return `${m}m`;
}