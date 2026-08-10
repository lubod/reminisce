import { useState } from "react";
import { observer } from "mobx-react-lite";
import type { SystemStore, Series } from "../../stores/SystemStore";

const W = 640;
const H = 150;
const PAD_X = 4;
const PAD_Y = 8;

const PALETTE = ["#60a5fa", "#34d399", "#fbbf24", "#f472b6", "#a78bfa", "#f87171", "#22d3ee"];

interface ChartSpec {
    title: string;
    names: string[];
    unit: string;
}

const CHARTS: ChartSpec[] = [
    { title: "System load", names: ["system_cpu_percent", "system_mem_percent", "system_disk_used_percent", "db_pool_util_percent"], unit: "%" },
    { title: "AI throughput", names: ["ai_descriptions_per_hr", "ai_embeddings_per_hr", "ai_faces_per_hr"], unit: "/hr" },
    { title: "AI latency p95", names: ["ai_description_p95_ms", "ai_embedding_p95_ms", "ai_face_p95_ms"], unit: "ms" },
    { title: "HTTP requests", names: ["http_requests_per_hr"], unit: "/hr" },
    { title: "HTTP latency p95", names: ["http_p95_ms"], unit: "ms" },
    { title: "Errors", names: ["ai_errors_per_hr"], unit: "/hr" },
    { title: "Pending backlog", names: ["backlog_description", "backlog_embedding", "backlog_face"], unit: "images" },
    { title: "Backup peers", names: ["backup_peers_available"], unit: "peers" },
];

function toSvg(series: Series, minT: number, maxT: number, minV: number, maxV: number): string {
    if (!series.points.length) return "";
    const x = (t: number) => PAD_X + ((t - minT) / (maxT - minT || 1)) * (W - PAD_X * 2);
    const y = (v: number) => H - PAD_Y - ((v - minV) / (maxV - minV || 1)) * (H - PAD_Y * 2);
    // polyline `points` is a space-separated list of "x,y" — no path M/L commands.
    return series.points.map((p) => `${x(p.t).toFixed(1)},${y(p.v).toFixed(1)}`).join(" ");
}

function ChartCard({ spec, store }: { spec: ChartSpec; store: SystemStore }) {
    const present = spec.names
        .map((n) => store.series.find((s) => s.name === n))
        .filter((s): s is Series => !!s && s.points.length > 0);

    if (present.length === 0) return null;

    let minV = Infinity;
    let maxV = -Infinity;
    let minT = Infinity;
    let maxT = -Infinity;
    for (const s of present) {
        for (const p of s.points) {
            if (p.v < minV) minV = p.v;
            if (p.v > maxV) maxV = p.v;
            if (p.t < minT) minT = p.t;
            if (p.t > maxT) maxT = p.t;
        }
    }
    if (!isFinite(minV)) return null;
    if (maxV === minV) {
        maxV += 1;
        minV -= 1;
    }
    if (maxT === minT) maxT += 1;

    const fmt = (v: number) => (Math.abs(v) >= 100 ? Math.round(v).toLocaleString() : v.toFixed(1));

    // Hover tooltip: track the time under the cursor and the nearest value per series.
    const [hoverT, setHoverT] = useState<number | null>(null);
    const xOf = (t: number) => PAD_X + ((t - minT) / (maxT - minT || 1)) * (W - PAD_X * 2);
    const timeOfX = (x: number) => minT + ((x - PAD_X) / (W - PAD_X * 2)) * (maxT - minT);

    const hoveredSeries = present
        .map((s) => {
            // nearest point in time
            let best = s.points[0];
            for (const p of s.points) if (Math.abs(p.t - hoverT!) < Math.abs(best.t - hoverT!)) best = p;
            return { s, p: best };
        })
        .sort((a, b) => (a.p.t - b.p.t) as number);

    const fmtTime = (t: number) =>
        new Date(t * 1000).toLocaleString(undefined, {
            month: "short",
            day: "numeric",
            hour: "2-digit",
            minute: "2-digit",
            second: "2-digit",
        });

    const handleMove = (e: React.MouseEvent<SVGSVGElement>) => {
        const rect = e.currentTarget.getBoundingClientRect();
        const fx = (e.clientX - rect.left) / rect.width;
        setHoverT(timeOfX(fx * W));
    };

    return (
        <div className="bg-gray-900/40 rounded-xl border border-gray-700 p-4">
            <div className="flex items-center justify-between mb-2">
                <span className="text-xs font-semibold text-gray-300">{spec.title}</span>
                <span className="text-[10px] text-gray-500">
                    min {fmt(minV)} · max {fmt(maxV)} {spec.unit}
                </span>
            </div>
            <div
                className="relative"
                onMouseLeave={() => setHoverT(null)}
            >
                <svg
                    viewBox={`0 0 ${W} ${H}`}
                    className="w-full h-36 cursor-crosshair"
                    preserveAspectRatio="none"
                    onMouseMove={handleMove}
                >
                    {hoverT !== null && (
                        <line
                            x1={xOf(hoverT)}
                            y1={PAD_Y}
                            x2={xOf(hoverT)}
                            y2={H - PAD_Y}
                            stroke="#e5e7eb"
                            strokeOpacity={0.5}
                            strokeWidth={1}
                            vectorEffect="non-scaling-stroke"
                        />
                    )}
                    {present.map((s, i) => (
                        <polyline
                            key={s.name}
                            points={toSvg(s, minT, maxT, minV, maxV)}
                            fill="none"
                            stroke={PALETTE[i % PALETTE.length]}
                            strokeWidth={1.5}
                            vectorEffect="non-scaling-stroke"
                        />
                    ))}
                    {hoverT !== null &&
                        hoveredSeries.map(({ s, p }) => (
                            <circle
                                key={s.name}
                                cx={xOf(p.t)}
                                cy={
                                    H -
                                    PAD_Y -
                                    ((p.v - minV) / (maxV - minV || 1)) * (H - PAD_Y * 2)
                                }
                                r={3}
                                fill={PALETTE[present.indexOf(s) % PALETTE.length]}
                                stroke="#0f172a"
                                strokeWidth={1}
                                vectorEffect="non-scaling-stroke"
                            />
                        ))}
                </svg>

                {hoverT !== null && (
                    <div
                        className="pointer-events-none absolute z-10 -translate-x-1/2 top-0 bg-gray-950/95 border border-gray-700 rounded-md px-2 py-1.5 shadow-lg"
                        style={{
                            left: `${(xOf(hoverT) / W) * 100}%`,
                            maxWidth: "15rem",
                        }}
                    >
                        <div className="text-[10px] text-gray-400 font-medium mb-0.5 whitespace-nowrap">
                            {fmtTime(hoverT)}
                        </div>
                        {hoveredSeries.map(({ s, p }) => (
                            <div
                                key={s.name}
                                className="flex items-center gap-1.5 text-[11px] text-gray-200 whitespace-nowrap"
                            >
                                <span
                                    className="w-2 h-2 rounded-full shrink-0"
                                    style={{
                                        background: PALETTE[present.indexOf(s) % PALETTE.length],
                                    }}
                                />
                                <span className="text-gray-400">{shortName(s.name)}</span>
                                <span className="ml-auto font-semibold tabular-nums">
                                    {fmt(p.v)} {spec.unit}
                                </span>
                            </div>
                        ))}
                    </div>
                )}
            </div>
            <div className="flex flex-wrap gap-x-3 gap-y-0.5 mt-1.5">
                {present.map((s, i) => (
                    <span key={s.name} className="flex items-center gap-1 text-[10px] text-gray-400">
                        <span className="w-2 h-2 rounded-full" style={{ background: PALETTE[i % PALETTE.length] }} />
                        {shortName(s.name)}
                    </span>
                ))}
            </div>
        </div>
    );
}

function shortName(name: string): string {
    const map: Record<string, string> = {
        system_cpu_percent: "CPU",
        system_mem_percent: "RAM",
        system_disk_used_percent: "Disk",
        db_pool_util_percent: "DB pool",
        ai_descriptions_per_hr: "Descriptions",
        ai_embeddings_per_hr: "Embeddings",
        ai_faces_per_hr: "Faces",
        ai_description_p95_ms: "Desc",
        ai_embedding_p95_ms: "Embed",
        ai_face_p95_ms: "Face",
        http_requests_per_hr: "Req",
        http_p95_ms: "HTTP",
        ai_errors_per_hr: "Errors",
        backlog_description: "Desc",
        backlog_embedding: "Embed",
        backlog_face: "Face",
        backup_peers_available: "Peers",
    };
    return map[name] ?? name;
}

const RANGES: { key: "1d" | "30d" | "90d"; label: string }[] = [
    { key: "1d", label: "1D" },
    { key: "30d", label: "30D" },
    { key: "90d", label: "90D" },
];

export const Charts = observer(({ store }: { store: SystemStore }) => {
    return (
        <div className="space-y-4">
            <div className="flex items-center gap-1">
                <span className="text-xs text-gray-400 mr-2">Range</span>
                {RANGES.map((r) => (
                    <button
                        key={r.key}
                        onClick={() => store.setRange(r.key)}
                        className={`px-3 py-1 rounded-md text-xs font-semibold border transition-colors ${
                            store.seriesRange === r.key
                                ? "bg-blue-600/30 border-blue-500 text-blue-200"
                                : "bg-gray-900/40 border-gray-700 text-gray-400 hover:text-gray-200"
                        }`}
                    >
                        {r.label}
                    </button>
                ))}
            </div>
            <div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-3">
                {CHARTS.map((spec) => (
                    <ChartCard key={spec.title} spec={spec} store={store} />
                ))}
            </div>
        </div>
    );
});