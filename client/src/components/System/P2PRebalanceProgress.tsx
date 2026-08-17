import React from "react";
import { observer } from "mobx-react-lite";
import { Shuffle, Layers, CheckCircle2, RefreshCw, HardDrive } from "lucide-react";
import { useStore } from "../../stores/RootStore";

export const P2PRebalanceProgress: React.FC = observer(() => {
    const { statsStore } = useStore();
    const p2pStatus = statsStore.p2pBackupStatus;
    const peers = statsStore.discoveredPeers;

    const rebalance = p2pStatus?.rebalance;
    const totalFiles = rebalance?.total_files ?? p2pStatus?.ok_files ?? 0;
    const balancedFiles = rebalance?.balanced_files ?? (rebalance?.progress_percent === 100 ? totalFiles : 0);
    const unbalancedFiles = rebalance?.unbalanced_files ?? 0;
    const progressPercent = Math.min(100, Math.max(0, rebalance?.progress_percent ?? (unbalancedFiles === 0 && totalFiles > 0 ? 100 : 0)));
    const targetNodes = rebalance?.target_nodes ?? (peers.filter(p => p.is_active).length || 1);
    const isActive = rebalance?.is_active ?? (unbalancedFiles > 0 && targetNodes > 1);

    const totalStoredShards = p2pStatus?.total_shards_stored ?? 0;
    const activePeers = peers.filter(p => p.is_active);

    return (
        <div className="bg-gray-900/60 rounded-2xl p-6 border border-gray-700 shadow-xl space-y-6">
            {/* Header */}
            <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
                <div className="flex items-center gap-3">
                    <div className="p-2.5 bg-amber-500/10 border border-amber-500/20 rounded-xl text-amber-400">
                        <Shuffle className="w-5 h-5" />
                    </div>
                    <div>
                        <h3 className="text-base font-bold text-white flex items-center gap-2">
                            Mesh Shard Distribution & Rebalance
                        </h3>
                        <p className="text-xs text-gray-400 mt-0.5">
                            Rendezvous hash migration across {targetNodes} active storage {targetNodes === 1 ? "node" : "nodes"}
                        </p>
                    </div>
                </div>

                <div className="flex items-center gap-2">
                    {progressPercent >= 100 ? (
                        <span className="inline-flex items-center gap-1.5 px-3 py-1.5 bg-emerald-950/60 text-emerald-400 border border-emerald-500/30 rounded-full text-xs font-bold shadow-sm">
                            <CheckCircle2 className="w-3.5 h-3.5" />
                            OPTIMAL (100%)
                        </span>
                    ) : isActive ? (
                        <span className="inline-flex items-center gap-1.5 px-3 py-1.5 bg-amber-950/60 text-amber-300 border border-amber-500/40 rounded-full text-xs font-bold shadow-sm animate-pulse">
                            <RefreshCw className="w-3.5 h-3.5 animate-spin text-amber-400" />
                            REBALANCING ({progressPercent.toFixed(1)}%)
                        </span>
                    ) : (
                        <span className="inline-flex items-center gap-1.5 px-3 py-1.5 bg-blue-950/60 text-blue-300 border border-blue-500/30 rounded-full text-xs font-bold shadow-sm">
                            <Layers className="w-3.5 h-3.5 text-blue-400" />
                            PENDING ({progressPercent.toFixed(1)}%)
                        </span>
                    )}
                </div>
            </div>

            {/* Progress Bar Container */}
            <div className="space-y-2">
                <div className="flex justify-between items-center text-xs font-semibold">
                    <span className="text-gray-300 flex items-center gap-1.5">
                        <span className="w-2 h-2 rounded-full bg-emerald-400 inline-block" />
                        Distribution Equilibrium Progress
                    </span>
                    <span className="font-mono text-sm font-bold text-white">
                        {progressPercent.toFixed(1)}%
                    </span>
                </div>

                <div className="w-full bg-gray-950/80 rounded-full h-4 overflow-hidden p-0.5 border border-gray-700/80 shadow-inner relative">
                    <div
                        className="h-full rounded-full transition-all duration-700 ease-out bg-gradient-to-r from-blue-600 via-indigo-500 to-emerald-500 relative overflow-hidden"
                        style={{ width: `${Math.max(1.5, progressPercent)}%` }}
                    >
                        {isActive && (
                            <div className="absolute inset-0 bg-white/20 animate-[shimmer_2s_infinite] bg-[linear-gradient(90deg,transparent_0%,rgba(255,255,255,0.3)_50%,transparent_100%)]" />
                        )}
                    </div>
                </div>
            </div>

            {/* Metric Chips */}
            <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
                <div className="bg-gray-800/60 rounded-xl p-3.5 border border-gray-700/60">
                    <div className="text-[10px] uppercase font-bold text-gray-400 tracking-wider">Balanced Files</div>
                    <div className="text-xl font-black text-emerald-400 mt-1">
                        {balancedFiles.toLocaleString()}
                    </div>
                    <div className="text-[10px] text-gray-500 mt-0.5">Spread on $\ge$ {targetNodes} nodes</div>
                </div>

                <div className="bg-gray-800/60 rounded-xl p-3.5 border border-gray-700/60">
                    <div className="text-[10px] uppercase font-bold text-gray-400 tracking-wider">Pending Rebalance</div>
                    <div className="text-xl font-black text-amber-400 mt-1">
                        {unbalancedFiles.toLocaleString()}
                    </div>
                    <div className="text-[10px] text-gray-500 mt-0.5">Awaiting migration</div>
                </div>

                <div className="bg-gray-800/60 rounded-xl p-3.5 border border-gray-700/60">
                    <div className="text-[10px] uppercase font-bold text-gray-400 tracking-wider">Total Library Files</div>
                    <div className="text-xl font-black text-white mt-1">
                        {totalFiles.toLocaleString()}
                    </div>
                    <div className="text-[10px] text-gray-500 mt-0.5">{totalStoredShards.toLocaleString()} shards total</div>
                </div>

                <div className="bg-gray-800/60 rounded-xl p-3.5 border border-gray-700/60">
                    <div className="text-[10px] uppercase font-bold text-gray-400 tracking-wider">Active Storage Nodes</div>
                    <div className="text-xl font-black text-blue-400 mt-1">
                        {activePeers.length} {activePeers.length === 1 ? "Node" : "Nodes"}
                    </div>
                    <div className="text-[10px] text-gray-500 mt-0.5">3/5 Reed-Solomon target</div>
                </div>
            </div>

            {/* Per-Node Live Allocation Bar */}
            {activePeers.length > 0 && totalStoredShards > 0 && (
                <div className="pt-4 border-t border-gray-700/50 space-y-3">
                    <div className="flex justify-between items-center text-xs">
                        <span className="text-gray-400 font-bold uppercase tracking-wider text-[10px] flex items-center gap-1.5">
                            <HardDrive className="w-3.5 h-3.5 text-gray-400" />
                            Live Shard Allocation by Storage Node
                        </span>
                        <span className="text-gray-500 text-[11px]">
                            {activePeers.length} active destinations
                        </span>
                    </div>

                    {/* Proportional Segmented Bar */}
                    <div className="w-full h-3 rounded-lg overflow-hidden flex bg-gray-950/80 p-0.5 gap-0.5 border border-gray-700/60">
                        {activePeers.map((peer, idx) => {
                            const pct = (peer.shard_count / Math.max(1, totalStoredShards)) * 100;
                            const colors = [
                                "bg-blue-500",
                                "bg-indigo-500",
                                "bg-emerald-500",
                                "bg-amber-500",
                                "bg-purple-500"
                            ];
                            const color = colors[idx % colors.length];
                            return (
                                <div
                                    key={peer.peer_id}
                                    title={`${peer.public_addr || peer.peer_id.slice(0, 12)}: ${peer.shard_count.toLocaleString()} shards (${pct.toFixed(1)}%)`}
                                    className={`${color} h-full rounded-sm transition-all duration-500`}
                                    style={{ width: `${Math.max(0.5, pct)}%` }}
                                />
                            );
                        })}
                    </div>

                    {/* Node Labels Grid */}
                    <div className="grid grid-cols-1 sm:grid-cols-3 gap-2 pt-1">
                        {activePeers.map((peer, idx) => {
                            const pct = (peer.shard_count / Math.max(1, totalStoredShards)) * 100;
                            const dotColors = [
                                "bg-blue-500",
                                "bg-indigo-500",
                                "bg-emerald-500",
                                "bg-amber-500",
                                "bg-purple-500"
                            ];
                            return (
                                <div key={peer.peer_id} className="flex items-center justify-between text-xs bg-gray-950/40 px-3 py-2 rounded-lg border border-gray-800">
                                    <div className="flex items-center gap-2 min-w-0">
                                        <span className={`w-2 h-2 rounded-full ${dotColors[idx % dotColors.length]} shrink-0`} />
                                        <span className="font-mono text-gray-300 truncate text-[11px]">
                                            {peer.public_addr || `${peer.peer_id.slice(0, 12)}...`}
                                        </span>
                                    </div>
                                    <span className="font-bold text-white font-mono text-[11px] shrink-0">
                                        {peer.shard_count.toLocaleString()} ({pct.toFixed(1)}%)
                                    </span>
                                </div>
                            );
                        })}
                    </div>
                </div>
            )}
        </div>
    );
});
