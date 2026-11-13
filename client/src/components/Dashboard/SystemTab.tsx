import { observer } from "mobx-react-lite";
import { Activity, AlertTriangle, Clock, Cpu, Database, HardDrive, MemoryStick, Network, ShieldAlert, TrendingUp, Zap } from "lucide-react";
import type { GeoDbStats, PoolMetrics, ServiceHealth, SystemStats } from "../../stores/StatsStore";
import type { SystemStore } from "../../stores/SystemStore";
import { AiModelsViewer } from "../System/AiModelsViewer";
import { ErrorsViewer } from "../System/ErrorsViewer";
import { Charts } from "../System/Charts";
import { LogViewer } from "../System/LogViewer";

interface SystemTabProps {
    systemStats: SystemStats | null;
    poolStats: PoolMetrics | undefined;
    serviceHealth: ServiceHealth | null;
    geoDbStats: GeoDbStats | null;
    systemStore: SystemStore;
}

function fmtMs(ms: number | null | undefined): string {
    if (ms === null || ms === undefined) return "—";
    return ms < 1000 ? `${ms.toFixed(0)} ms` : `${(ms / 1000).toFixed(2)} s`;
}

export const SystemTab = observer(({
    systemStats,
    poolStats,
    serviceHealth,
    geoDbStats,
    systemStore,
}: SystemTabProps) => {
    const dot = (status: string) =>
        status === "connected"
            ? <span className="inline-block w-2.5 h-2.5 rounded-full bg-green-400 mr-2" />
            : <span className="inline-block w-2.5 h-2.5 rounded-full bg-red-500 mr-2" />;

    return (
        <div className="space-y-6">
            <AiModelsViewer />
            <div className="bg-gray-800 shadow-xl rounded-xl p-8 border border-gray-700">
                <h2 className="text-xl font-bold text-gray-100 mb-6 flex items-center"><ShieldAlert className="w-6 h-6 text-amber-400 mr-3" /> Health Alerts</h2>
                {systemStore.alerts.length === 0 ? (
                    <div className="text-gray-500 italic">No alert data yet.</div>
                ) : (
                    <div className="grid gap-2 md:grid-cols-2 xl:grid-cols-3">
                        {systemStore.alerts.map((a) => (
                            <div
                                key={a.id}
                                className={`flex items-start gap-3 rounded-lg border px-4 py-3 text-sm ${
                                    a.status === 'firing'
                                        ? a.severity === 'critical'
                                            ? 'bg-red-900/40 border-red-700 text-red-200'
                                            : 'bg-amber-900/40 border-amber-700 text-amber-200'
                                        : 'bg-emerald-900/30 border-emerald-800 text-emerald-200'
                                }`}
                            >
                                <span className={`mt-1.5 w-2 h-2 rounded-full shrink-0 ${a.status === 'firing' ? (a.severity === 'critical' ? 'bg-red-500' : 'bg-amber-400') : 'bg-emerald-400'}`} />
                                <div className="min-w-0">
                                    <div className="font-semibold">{a.message}</div>
                                    <div className="text-xs opacity-80 truncate">{a.detail}</div>
                                </div>
                            </div>
                        ))}
                    </div>
                )}
            </div>

            <div className="bg-gray-800 shadow-xl rounded-xl p-8 border border-gray-700">
                <h2 className="text-xl font-bold text-gray-100 mb-6 flex items-center"><AlertTriangle className="w-6 h-6 text-red-400 mr-3" /> Recent Errors & Telemetry</h2>
                <ErrorsViewer />
            </div>

            <div className="bg-gray-800 shadow-xl rounded-xl p-8 border border-gray-700">
                <h2 className="text-xl font-bold text-gray-100 mb-8 flex items-center"><Activity className="w-6 h-6 text-blue-400 mr-3" /> System Resources</h2>
                {systemStats ? (
                    <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-8">
                        <div className="p-4 bg-gray-900/50 rounded-xl border border-gray-700">
                            <dt className="text-xs font-black text-gray-500 uppercase mb-3 flex items-center gap-2"><Cpu className="w-4 h-4 text-blue-400" /> CPU</dt>
                            <dd className="text-3xl font-black text-white">{systemStats.cpu_usage_percent.toFixed(1)}%</dd>
                            {systemStats.cpu_temp_celsius !== null && <dd className="text-[10px] text-gray-500 mt-1 font-bold">{systemStats.cpu_temp_celsius.toFixed(0)}°C</dd>}
                        </div>
                        <div className="p-4 bg-gray-900/50 rounded-xl border border-gray-700">
                            <dt className="text-xs font-black text-gray-500 uppercase mb-3 flex items-center gap-2"><MemoryStick className="w-4 h-4 text-purple-400" /> RAM</dt>
                            <dd className="text-3xl font-black text-white">{systemStats.memory_usage_percent.toFixed(1)}%</dd>
                            <dd className="text-[10px] text-gray-500 mt-1 font-bold">{systemStats.memory_used_gb.toFixed(1)} GB used</dd>
                        </div>
                        <div className="p-4 bg-gray-900/50 rounded-xl border border-gray-700">
                            <dt className="text-xs font-black text-gray-500 uppercase mb-3 flex items-center gap-2"><HardDrive className="w-4 h-4 text-green-400" /> Disk</dt>
                            <dd className="text-3xl font-black text-white">{systemStats.disk_usage_percent.toFixed(1)}%</dd>
                            <dd className="text-[10px] text-gray-500 mt-1 font-bold">{systemStats.disk_available_gb.toFixed(1)} GB free</dd>
                        </div>
                        <div className="p-4 bg-gray-900/50 rounded-xl border border-gray-700">
                            <dt className="text-xs font-black text-gray-500 uppercase mb-3 flex items-center gap-2"><Zap className="w-4 h-4 text-yellow-400" /> GPU</dt>
                            <dd className="text-3xl font-black text-white">{systemStats.gpu_usage_percent !== null ? `${systemStats.gpu_usage_percent.toFixed(1)}%` : 'N/A'}</dd>
                            <dd className="text-[10px] text-gray-500 mt-1 font-bold">
                                {systemStats.gpu_memory_used_mb !== null && systemStats.gpu_memory_total_mb !== null
                                    ? `${(systemStats.gpu_memory_used_mb / 1024).toFixed(1)} / ${(systemStats.gpu_memory_total_mb / 1024).toFixed(0)} GB VRAM`
                                    : systemStats.gpu_info || 'No GPU detected'}
                            </dd>
                            {systemStats.gpu_temp_celsius !== null && <dd className="text-[10px] text-gray-500 mt-1 font-bold">{systemStats.gpu_temp_celsius.toFixed(0)}°C</dd>}
                        </div>
                        <div className="p-4 bg-gray-900/50 rounded-xl border border-gray-700">
                            <dt className="text-xs font-black text-gray-500 uppercase mb-3 flex items-center gap-2"><Clock className="w-4 h-4 text-orange-400" /> Uptime</dt>
                            <dd className="text-2xl font-black text-white">{Math.floor(systemStats.uptime_seconds / 3600)}h {Math.floor((systemStats.uptime_seconds % 3600) / 60)}m</dd>
                        </div>
                    </div>
                ) : <div className="text-gray-500 italic">Waiting for system telemetry...</div>}
            </div>

            <div className="bg-gray-800 shadow-xl rounded-xl p-8 border border-gray-700">
                <h2 className="text-xl font-bold text-gray-100 mb-8 flex items-center"><Database className="w-6 h-6 text-green-400 mr-3" /> Services</h2>
                <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
                    <div className="flex items-center p-4 bg-gray-900/50 rounded-xl border border-gray-700">
                        {serviceHealth ? dot(serviceHealth.database) : <span className="inline-block w-2.5 h-2.5 rounded-full bg-gray-500 mr-2" />}
                        <span className="text-sm font-semibold text-gray-300">Main Database</span>
                        <span className="ml-auto text-xs text-gray-500">{serviceHealth?.database ?? '—'}</span>
                    </div>
                    <div className="flex items-center p-4 bg-gray-900/50 rounded-xl border border-gray-700">
                        {serviceHealth ? dot(serviceHealth.geotagging_database) : <span className="inline-block w-2.5 h-2.5 rounded-full bg-gray-500 mr-2" />}
                        <span className="text-sm font-semibold text-gray-300">Geotagging DB</span>
                        <span className="ml-auto text-xs text-gray-500">{serviceHealth?.geotagging_database ?? '—'}</span>
                    </div>
                    <div className="flex items-center p-4 bg-gray-900/50 rounded-xl border border-gray-700">
                        {serviceHealth ? dot(serviceHealth.ai_service) : <span className="inline-block w-2.5 h-2.5 rounded-full bg-gray-500 mr-2" />}
                        <span className="text-sm font-semibold text-gray-300">AI Service</span>
                        <span className="ml-auto text-xs text-gray-500">{serviceHealth?.ai_service ?? '—'}</span>
                    </div>
                </div>
            </div>

            <div className="bg-gray-800 shadow-xl rounded-xl p-8 border border-gray-700">
                <h2 className="text-xl font-bold text-gray-100 mb-6 flex items-center"><Database className="w-6 h-6 text-teal-400 mr-3" /> Geo Database</h2>
                {geoDbStats ? (
                    <div className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-7 gap-4">
                        <div className="p-4 bg-gray-900/50 rounded-xl border border-gray-700 text-center">
                            <div className="text-[10px] text-gray-500 uppercase font-bold mb-1">Total</div>
                            <div className="text-2xl font-black text-white">{geoDbStats.total_boundaries.toLocaleString()}</div>
                        </div>
                        <div className="p-4 bg-gray-900/50 rounded-xl border border-gray-700 text-center">
                            <div className="text-[10px] text-gray-500 uppercase font-bold mb-1">Countries</div>
                            <div className="text-2xl font-black text-blue-400">{geoDbStats.countries.toLocaleString()}</div>
                        </div>
                        <div className="p-4 bg-gray-900/50 rounded-xl border border-gray-700 text-center">
                            <div className="text-[10px] text-gray-500 uppercase font-bold mb-1">States</div>
                            <div className="text-2xl font-black text-purple-400">{geoDbStats.states_provinces.toLocaleString()}</div>
                        </div>
                        <div className="p-4 bg-gray-900/50 rounded-xl border border-gray-700 text-center">
                            <div className="text-[10px] text-gray-500 uppercase font-bold mb-1">Counties</div>
                            <div className="text-2xl font-black text-indigo-400">{geoDbStats.counties.toLocaleString()}</div>
                        </div>
                        <div className="p-4 bg-gray-900/50 rounded-xl border border-gray-700 text-center">
                            <div className="text-[10px] text-gray-500 uppercase font-bold mb-1">Cities</div>
                            <div className="text-2xl font-black text-teal-400">{geoDbStats.cities.toLocaleString()}</div>
                        </div>
                        <div className="p-4 bg-gray-900/50 rounded-xl border border-gray-700 text-center">
                            <div className="text-[10px] text-gray-500 uppercase font-bold mb-1">Other</div>
                            <div className="text-2xl font-black text-gray-400">{geoDbStats.other_boundaries.toLocaleString()}</div>
                        </div>
                        <div className="p-4 bg-gray-900/50 rounded-xl border border-gray-700 text-center">
                            <div className="text-[10px] text-gray-500 uppercase font-bold mb-1">Unique Countries</div>
                            <div className="text-2xl font-black text-green-400">{geoDbStats.unique_countries.toLocaleString()}</div>
                        </div>
                    </div>
                ) : <div className="text-gray-500 italic">Loading geo database stats...</div>}
            </div>

            <div className="bg-gray-800 shadow-xl rounded-xl p-8 border border-gray-700">
                <h2 className="text-xl font-bold text-gray-100 mb-8 flex items-center"><Database className="w-6 h-6 text-indigo-400 mr-3" /> Database Pool</h2>
                {poolStats ? (
                    <div className="grid grid-cols-1 md:grid-cols-3 gap-8">
                        <div className="border-l-4 border-blue-500 pl-6"><dt className="text-xs font-black text-gray-500 uppercase mb-1">Active</dt><dd className="text-3xl font-black text-white">{poolStats.size - Math.max(0, poolStats.available)} / {poolStats.max_size}</dd></div>
                        <div className="border-l-4 border-green-500 pl-6"><dt className="text-xs font-black text-gray-500 uppercase mb-1">Utilization</dt><dd className="text-3xl font-black text-white">{poolStats.utilization_percent.toFixed(1)}%</dd></div>
                        <div className="border-l-4 border-indigo-500 pl-6"><dt className="text-xs font-black text-gray-500 uppercase mb-1">Free Connections</dt><dd className="text-3xl font-black text-white">{poolStats.available}</dd></div>
                    </div>
                ) : <div className="text-gray-500 italic">Connecting to pool metrics...</div>}
            </div>

            <div className="bg-gray-800 shadow-xl rounded-xl p-8 border border-gray-700">
                <h2 className="text-xl font-bold text-gray-100 mb-6 flex items-center"><Zap className="w-6 h-6 text-yellow-400 mr-3" /> Pipeline Timing</h2>
                {systemStore.pipeline ? (
                    <div className="overflow-x-auto">
                        <table className="w-full text-sm">
                            <thead>
                                <tr className="text-left text-gray-500 uppercase text-xs">
                                    <th className="pb-2 pr-4">Worker</th>
                                    <th className="pb-2 pr-4">Samples</th>
                                    <th className="pb-2 pr-4">mean</th>
                                    <th className="pb-2 pr-4">p50</th>
                                    <th className="pb-2 pr-4">p90</th>
                                    <th className="pb-2 pr-4">p95</th>
                                    <th className="pb-2">p99</th>
                                </tr>
                            </thead>
                            <tbody>
                                {[...systemStore.pipeline.workers, systemStore.pipeline.db_query_ms].filter(Boolean).map((w) => (
                                    <tr key={w.id} className="border-b border-gray-700/60">
                                        <td className="py-2 pr-4 text-gray-200 font-medium">{w.name}</td>
                                        <td className="py-2 pr-4 text-gray-400 font-mono">{w.count.toLocaleString()}</td>
                                        <td className="py-2 pr-4 text-gray-200 font-mono">{fmtMs(w.mean_ms)}</td>
                                        <td className="py-2 pr-4 text-gray-200 font-mono">{fmtMs(w.p50_ms)}</td>
                                        <td className="py-2 pr-4 text-gray-200 font-mono">{fmtMs(w.p90_ms)}</td>
                                        <td className="py-2 pr-4 text-gray-200 font-mono">{fmtMs(w.p95_ms)}</td>
                                        <td className="py-2 text-amber-300 font-mono">{fmtMs(w.p99_ms)}</td>
                                    </tr>
                                ))}
                            </tbody>
                        </table>
                    </div>
                ) : (
                    <div className="text-gray-500 italic">Waiting for pipeline telemetry...</div>
                )}
            </div>

            <div className="bg-gray-800 shadow-xl rounded-xl p-8 border border-gray-700">
                <h2 className="text-xl font-bold text-gray-100 mb-6 flex items-center"><Network className="w-6 h-6 text-blue-400 mr-3" /> HTTP Traffic</h2>
                {systemStore.pipeline ? (() => {
                    const h = systemStore.pipeline.http;
                    const statuses = [
                        { label: '2xx', value: h.status.http_2xx, cls: 'bg-green-500' },
                        { label: '3xx', value: h.status.http_3xx, cls: 'bg-blue-500' },
                        { label: '4xx', value: h.status.http_4xx, cls: 'bg-amber-500' },
                        { label: '5xx', value: h.status.http_5xx, cls: 'bg-red-500' },
                    ];
                    const max = Math.max(1, ...statuses.map((s) => s.value));
                    return (
                        <div className="space-y-4">
                            <div className="grid grid-cols-1 md:grid-cols-4 gap-6">
                                <div className="border-l-4 border-blue-500 pl-4">
                                    <dt className="text-xs font-black text-gray-500 uppercase mb-1">Total requests</dt>
                                    <dd className="text-2xl font-black text-white">{h.total.toLocaleString()}</dd>
                                </div>
                                <div className="border-l-4 border-indigo-500 pl-4">
                                    <dt className="text-xs font-black text-gray-500 uppercase mb-1">Requests / sec</dt>
                                    <dd className="text-2xl font-black text-white">{h.per_second.toFixed(2)}</dd>
                                </div>
                                <div className="border-l-4 border-purple-500 pl-4">
                                    <dt className="text-xs font-black text-gray-500 uppercase mb-1">HTTP p95</dt>
                                    <dd className="text-2xl font-black text-white">{fmtMs(h.duration_ms.p95_ms)}</dd>
                                </div>
                                <div className="border-l-4 border-teal-500 pl-4">
                                    <dt className="text-xs font-black text-gray-500 uppercase mb-1">HTTP p99</dt>
                                    <dd className="text-2xl font-black text-white">{fmtMs(h.duration_ms.p99_ms)}</dd>
                                </div>
                            </div>
                            <div className="space-y-2">
                                {statuses.map((s) => (
                                    <div key={s.label} className="flex items-center gap-3">
                                        <span className="w-8 text-xs text-gray-400">{s.label}</span>
                                        <div className="flex-1 h-2.5 bg-gray-900/60 rounded-full overflow-hidden">
                                            <div className={`h-full rounded-full ${s.cls}`} style={{ width: `${(s.value / max) * 100}%` }} />
                                        </div>
                                        <span className="w-20 text-right text-xs font-mono text-gray-300">{s.value.toLocaleString()}</span>
                                    </div>
                                ))}
                            </div>
                            <p className="text-[11px] text-gray-500">
                                Duration percentiles: p50 {fmtMs(h.duration_ms.p50_ms)} · p90 {fmtMs(h.duration_ms.p90_ms)} · p95 {fmtMs(h.duration_ms.p95_ms)} · p99 {fmtMs(h.duration_ms.p99_ms)}
                            </p>
                        </div>
                    );
                })() : (
                    <div className="text-gray-500 italic">Waiting for HTTP telemetry...</div>
                )}
            </div>

            <div className="bg-gray-800 shadow-xl rounded-xl p-8 border border-gray-700">
                <h2 className="text-xl font-bold text-gray-100 mb-6 flex items-center"><TrendingUp className="w-6 h-6 text-blue-400 mr-3" /> History (1D / 30D / 90D)</h2>
                <Charts store={systemStore} />
            </div>

            <div className="bg-gray-800 shadow-xl rounded-xl p-8 border border-gray-700">
                <h2 className="text-xl font-bold text-gray-100 mb-6 flex items-center"><Activity className="w-6 h-6 text-teal-400 mr-3" /> Live Logs</h2>
                <LogViewer />
            </div>
        </div>
    );
});
