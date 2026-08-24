import { useEffect, useMemo, useRef, useState } from "react";
import { observer } from "mobx-react-lite";
import { useStore } from "../../stores/RootStore";
import type { LogLine } from "../../stores/SystemStore";

const levelColor: Record<string, string> = {
    ERROR: "text-red-400",
    PANIC: "text-red-300 bg-red-950/60 font-bold",
    WARN: "text-amber-400",
    WARNING: "text-amber-400",
    INFO: "text-gray-300",
    DEBUG: "text-gray-500",
    TRACE: "text-gray-600",
};

function fmtTime(ts: number): string {
    if (!ts) return "";
    const d = new Date(ts * 1000);
    return d.toISOString();
}

export const LogViewer = observer(() => {
    const { systemStore } = useStore();
    const [level, setLevel] = useState<string>("info");

    const filterRank = (lvl: string) => {
        switch (lvl) {
            case "error":
                return 4;
            case "warn":
                return 3;
            case "info":
                return 2;
            case "debug":
                return 1;
            default:
                return 0;
        }
    };
    const entryRank = (lvl: string) => {
        if (lvl === "PANIC") return 5;
        switch (lvl) {
            case "ERROR":
                return 4;
            case "WARN":
            case "WARNING":
                return 3;
            case "INFO":
                return 2;
            case "DEBUG":
                return 1;
            default:
                return 0;
        }
    };

    const min = filterRank(level);
    const filtered = useMemo(
        () => systemStore.logs.filter((l) => entryRank(l.level) >= min),
        [systemStore.logs, min]
    );

    // Cap the DOM window: render only the most recent 200 matching lines.
    const visible = useMemo(() => filtered.slice(-200), [filtered]);
    const scrollRef = useRef<HTMLDivElement>(null);

    // Autoscroll to the newest line when the window updates.
    useEffect(() => {
        const el = scrollRef.current;
        if (el) el.scrollTop = el.scrollHeight;
    }, [visible]);

    return (
        <div className="flex flex-col gap-2">
            <div className="flex items-center gap-2">
                <label className="text-xs text-gray-400">Level</label>
                <select
                    className="bg-gray-900 border border-gray-700 rounded px-2 py-1 text-sm text-gray-200"
                    value={level}
                    onChange={(e) => setLevel(e.target.value)}
                >
                    <option value="debug">debug</option>
                    <option value="info">info</option>
                    <option value="warn">warn</option>
                    <option value="error">error</option>
                </select>
                <span className="text-xs text-gray-500 ml-auto">
                    {filtered.length > visible.length && `last ${visible.length} of `}
                    {filtered.length} lines
                </span>
            </div>
            <div ref={scrollRef} className="bg-black/40 border border-gray-800 rounded-lg overflow-hidden h-80 overflow-y-auto font-mono text-xs">
                {visible.length === 0 ? (
                    <div className="p-3 text-gray-600">No log lines yet…</div>
                ) : (
                    visible.map((line: LogLine, i: number) => (
                        <div key={`${line.timestamp}-${i}`} className="flex gap-2 px-3 py-0.5 border-b border-gray-900/60">
                            <span className="text-gray-600 shrink-0">{fmtTime(line.timestamp).slice(11, 23)}</span>
                            <span className={`shrink-0 w-12 ${levelColor[line.level] ?? "text-gray-300"}`}>{line.level}</span>
                            <span className="text-gray-500 shrink-0">{line.target}</span>
                            <span className={`whitespace-pre-wrap break-all ${levelColor[line.level] ?? "text-gray-200"}`}>
                                {line.message}
                            </span>
                        </div>
                    ))
                )}
            </div>
        </div>
    );
});