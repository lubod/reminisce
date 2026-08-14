import React, { useState, useMemo } from "react";
import { observer } from "mobx-react-lite";
import { useStore } from "../../stores/RootStore";
import type { LogLine } from "../../stores/SystemStore";
import { AlertTriangle, AlertCircle, ShieldAlert, CheckCircle2, ChevronDown, ChevronRight, Copy, Check, Filter, Search } from "lucide-react";

interface ErrorGroup {
    key: string;
    level: string;
    target: string;
    sampleMessage: string;
    count: number;
    firstSeen: number;
    lastSeen: number;
    instances: LogLine[];
}

function timeAgo(ts: number): string {
    if (!ts) return "";
    const now = Date.now() / 1000;
    const diff = Math.max(0, Math.floor(now - ts));
    if (diff < 60) return `${diff}s ago`;
    if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
    return `${Math.floor(diff / 3600)}h ago`;
}

export const ErrorsViewer: React.FC = observer(() => {
    const { systemStore } = useStore();
    const [selectedLevel, setSelectedLevel] = useState<"all" | "error" | "warn" | "panic">("all");
    const [searchQuery, setSearchQuery] = useState("");
    const [expandedKey, setExpandedKey] = useState<string | null>(null);
    const [copiedKey, setCopiedKey] = useState<string | null>(null);

    // Group repeating error messages to eliminate spam
    const errorGroups = useMemo(() => {
        const groups: Map<string, ErrorGroup> = new Map();

        for (const err of systemStore.errors) {
            // Normalize message for clustering (strip specific hashes/uuids/numbers)
            const normalizedMsg = err.message
                .replace(/[0-9a-f]{64}/gi, "<hash>")
                .replace(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gi, "<uuid>")
                .replace(/\b\d+\b/g, "<num>")
                .trim();

            const key = `${err.level}:${err.target}:${normalizedMsg}`;

            if (groups.has(key)) {
                const grp = groups.get(key)!;
                grp.count += 1;
                grp.lastSeen = Math.max(grp.lastSeen, err.timestamp);
                grp.firstSeen = Math.min(grp.firstSeen, err.timestamp);
                grp.instances.push(err);
            } else {
                groups.set(key, {
                    key,
                    level: err.level.toUpperCase(),
                    target: err.target,
                    sampleMessage: err.message,
                    count: 1,
                    firstSeen: err.timestamp,
                    lastSeen: err.timestamp,
                    instances: [err],
                });
            }
        }

        return Array.from(groups.values()).sort((a, b) => b.lastSeen - a.lastSeen);
    }, [systemStore.errors]);

    const filteredGroups = useMemo(() => {
        return errorGroups.filter((g) => {
            if (selectedLevel === "error" && g.level !== "ERROR") return false;
            if (selectedLevel === "warn" && g.level !== "WARN" && g.level !== "WARNING") return false;
            if (selectedLevel === "panic" && g.level !== "PANIC") return false;

            if (searchQuery.trim()) {
                const q = searchQuery.toLowerCase();
                return g.sampleMessage.toLowerCase().includes(q) || g.target.toLowerCase().includes(q);
            }

            return true;
        });
    }, [errorGroups, selectedLevel, searchQuery]);

    const handleCopy = (key: string, text: string) => {
        navigator.clipboard.writeText(text);
        setCopiedKey(key);
        setTimeout(() => setCopiedKey(null), 2000);
    };

    return (
        <div className="space-y-4">
            {/* Header / Stats row */}
            <div className="grid grid-cols-1 sm:grid-cols-3 gap-4">
                <button
                    type="button"
                    onClick={() => setSelectedLevel(selectedLevel === "error" ? "all" : "error")}
                    className={`p-4 rounded-xl border text-left transition-all ${
                        selectedLevel === "error"
                            ? "bg-red-950/80 border-red-500 ring-1 ring-red-500"
                            : "bg-red-950/30 border-red-900/50 hover:bg-red-950/50"
                    }`}
                >
                    <div className="flex items-center justify-between">
                        <span className="text-xs uppercase tracking-wider font-bold text-red-400">Errors</span>
                        <AlertCircle className="w-4 h-4 text-red-400" />
                    </div>
                    <div className="text-2xl font-black text-red-300 mt-1">{systemStore.errorCounts.error}</div>
                    <div className="text-[10px] text-gray-500 font-medium mt-0.5">Last 5 minutes</div>
                </button>

                <button
                    type="button"
                    onClick={() => setSelectedLevel(selectedLevel === "warn" ? "all" : "warn")}
                    className={`p-4 rounded-xl border text-left transition-all ${
                        selectedLevel === "warn"
                            ? "bg-amber-950/80 border-amber-500 ring-1 ring-amber-500"
                            : "bg-amber-950/30 border-amber-900/50 hover:bg-amber-950/50"
                    }`}
                >
                    <div className="flex items-center justify-between">
                        <span className="text-xs uppercase tracking-wider font-bold text-amber-400">Warnings</span>
                        <AlertTriangle className="w-4 h-4 text-amber-400" />
                    </div>
                    <div className="text-2xl font-black text-amber-300 mt-1">{systemStore.errorCounts.warn}</div>
                    <div className="text-[10px] text-gray-500 font-medium mt-0.5">Last 5 minutes</div>
                </button>

                <button
                    type="button"
                    onClick={() => setSelectedLevel(selectedLevel === "panic" ? "all" : "panic")}
                    className={`p-4 rounded-xl border text-left transition-all ${
                        selectedLevel === "panic"
                            ? "bg-purple-950/80 border-purple-500 ring-1 ring-purple-500"
                            : "bg-purple-950/30 border-purple-900/50 hover:bg-purple-950/50"
                    }`}
                >
                    <div className="flex items-center justify-between">
                        <span className="text-xs uppercase tracking-wider font-bold text-purple-400">Panics</span>
                        <ShieldAlert className="w-4 h-4 text-purple-400" />
                    </div>
                    <div className="text-2xl font-black text-purple-300 mt-1">{systemStore.errorCounts.panic}</div>
                    <div className="text-[10px] text-gray-500 font-medium mt-0.5">Last 5 minutes</div>
                </button>
            </div>

            {/* Filter & Search Bar */}
            <div className="flex flex-wrap items-center gap-3 pt-2">
                <div className="relative flex-1 min-w-[200px]">
                    <Search className="w-4 h-4 absolute left-3 top-1/2 -translate-y-1/2 text-gray-500" />
                    <input
                        type="text"
                        placeholder="Search recent errors & targets..."
                        value={searchQuery}
                        onChange={(e) => setSearchQuery(e.target.value)}
                        className="w-full bg-gray-900/80 border border-gray-700/80 rounded-lg pl-9 pr-3 py-1.5 text-xs text-gray-200 placeholder-gray-500 focus:outline-none focus:border-blue-500"
                    />
                </div>

                <div className="flex items-center gap-1 bg-gray-900/60 p-1 rounded-lg border border-gray-800">
                    <Filter className="w-3.5 h-3.5 text-gray-500 ml-1.5" />
                    {(["all", "error", "warn", "panic"] as const).map((lvl) => (
                        <button
                            key={lvl}
                            type="button"
                            onClick={() => setSelectedLevel(lvl)}
                            className={`px-2.5 py-1 rounded text-[11px] font-semibold transition-all ${
                                selectedLevel === lvl
                                    ? "bg-blue-600 text-white shadow"
                                    : "text-gray-400 hover:text-gray-200"
                            }`}
                        >
                            {lvl.toUpperCase()}
                        </button>
                    ))}
                </div>
            </div>

            {/* Errors List */}
            <div className="space-y-2 pt-1">
                {filteredGroups.length === 0 ? (
                    <div className="flex flex-col items-center justify-center py-10 px-4 bg-gray-900/30 rounded-xl border border-dashed border-gray-800 text-center">
                        <CheckCircle2 className="w-8 h-8 text-emerald-400 mb-2 opacity-80" />
                        <div className="text-sm font-semibold text-gray-200">System Operating Cleanly</div>
                        <div className="text-xs text-gray-500 mt-0.5">
                            {systemStore.errors.length === 0
                                ? "Zero ERROR or PANIC events detected in the active telemetry window."
                                : "No errors match the current filter/search criteria."}
                        </div>
                    </div>
                ) : (
                    filteredGroups.map((group) => {
                        const isExpanded = expandedKey === group.key;
                        const isCopied = copiedKey === group.key;

                        const badgeColor =
                            group.level === "PANIC"
                                ? "bg-purple-900/40 text-purple-300 border-purple-700"
                                : group.level === "ERROR"
                                ? "bg-red-900/40 text-red-300 border-red-700"
                                : "bg-amber-900/40 text-amber-300 border-amber-700";

                        return (
                            <div
                                key={group.key}
                                className="bg-gray-900/50 border border-gray-800 rounded-xl overflow-hidden hover:border-gray-700 transition-colors"
                            >
                                <div
                                    className="p-3.5 flex items-start gap-3 cursor-pointer select-none"
                                    onClick={() => setExpandedKey(isExpanded ? null : group.key)}
                                >
                                    <button
                                        type="button"
                                        className="mt-0.5 text-gray-500 hover:text-gray-300 shrink-0"
                                        aria-label="Toggle error details"
                                    >
                                        {isExpanded ? <ChevronDown className="w-4 h-4" /> : <ChevronRight className="w-4 h-4" />}
                                    </button>

                                    <div className="flex-1 min-w-0">
                                        <div className="flex flex-wrap items-center gap-2 mb-1.5">
                                            <span className={`px-2 py-0.5 text-[10px] font-black uppercase rounded border ${badgeColor}`}>
                                                {group.level}
                                            </span>

                                            {group.count > 1 && (
                                                <span className="px-2 py-0.5 text-[10px] font-extrabold bg-red-950 text-red-400 border border-red-800 rounded-full">
                                                    {group.count}× occurrences
                                                </span>
                                            )}

                                            <span className="text-[11px] font-mono text-gray-400 bg-gray-800/80 px-2 py-0.5 rounded border border-gray-700/50">
                                                {group.target || "backend"}
                                            </span>

                                            <span className="text-[11px] text-gray-500 ml-auto font-medium">
                                                {timeAgo(group.lastSeen)}
                                            </span>
                                        </div>

                                        <p className="text-xs font-mono text-gray-200 break-words leading-relaxed">
                                            {group.sampleMessage}
                                        </p>
                                    </div>

                                    <button
                                        type="button"
                                        onClick={(e) => {
                                            e.stopPropagation();
                                            handleCopy(group.key, group.sampleMessage);
                                        }}
                                        className="p-1.5 text-gray-500 hover:text-gray-300 hover:bg-gray-800 rounded transition-colors shrink-0"
                                        title="Copy error message"
                                    >
                                        {isCopied ? <Check className="w-3.5 h-3.5 text-green-400" /> : <Copy className="w-3.5 h-3.5" />}
                                    </button>
                                </div>

                                {isExpanded && (
                                    <div className="bg-black/30 px-4 py-3 border-t border-gray-800/80 text-[11px] font-mono space-y-2">
                                        <div className="text-gray-400 font-bold uppercase text-[10px] tracking-wider mb-1">Recent Occurrences ({group.instances.length})</div>
                                        <div className="max-h-48 overflow-y-auto space-y-1.5 pr-2">
                                            {group.instances.map((inst, idx) => (
                                                <div key={`${inst.timestamp}-${idx}`} className="flex items-start gap-3 py-1 border-b border-gray-800/40 last:border-0">
                                                    <span className="text-gray-500 shrink-0 font-sans">{new Date(inst.timestamp * 1000).toLocaleTimeString()}</span>
                                                    <span className="text-gray-300 break-all">{inst.message}</span>
                                                </div>
                                            ))}
                                        </div>
                                    </div>
                                )}
                            </div>
                        );
                    })
                )}
            </div>
        </div>
    );
});
