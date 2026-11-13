import { Activity, AlertTriangle, Database, Image, Network, RefreshCw, Server, Shield, Shuffle, Trash2, Video } from "lucide-react";
import type { DashboardStats, DiscoveredPeer, P2PBackupStatus } from "../../stores/StatsStore";
import { P2PRebalanceProgress } from "../System/P2PRebalanceProgress";

interface BackupTabProps {
    stats: DashboardStats | null;
    p2pStatus: P2PBackupStatus | null;
    discoveredPeers: DiscoveredPeer[];
    isVerifying: boolean;
    isRebalancing: boolean;
    isLoadingStatus: boolean;
    onVerifyBackup: () => void;
    onForceRebalance: () => void;
    onRefreshStatus: () => void;
    onRemoveNode: (peerId: string, shardCount: number) => void;
}

function formatBytes(b: number): string {
    if (!b || b <= 0) return "0 B";
    const units = ["B", "KB", "MB", "GB", "TB"];
    const i = Math.min(Math.floor(Math.log(b) / Math.log(1024)), units.length - 1);
    return `${(b / Math.pow(1024, i)).toFixed(1)} ${units[i]}`;
}

export function BackupTab({
    stats,
    p2pStatus,
    discoveredPeers,
    isVerifying,
    isRebalancing,
    isLoadingStatus,
    onVerifyBackup,
    onForceRebalance,
    onRefreshStatus,
    onRemoveNode,
}: BackupTabProps) {
    return (
        <div className="bg-gray-800/80 backdrop-blur-md shadow-2xl rounded-2xl border border-gray-700 overflow-hidden">
            {/* Header */}
            <div className="px-8 py-6 border-b border-gray-700 bg-gray-800/50 flex justify-between items-center">
                <div>
                    <h2 className="text-2xl font-bold text-gray-100 flex items-center">
                        <Network className="w-7 h-7 text-blue-400 mr-3" /> P2P Backup
                    </h2>
                    <p className="text-gray-500 text-sm mt-1">Distributed 3/5 Reed-Solomon Sharding</p>
                </div>
                <div className="flex items-center gap-3">
                    <button onClick={onVerifyBackup} disabled={isVerifying} className="flex items-center gap-2 px-5 py-2.5 rounded-xl font-bold bg-emerald-600 hover:bg-emerald-500 text-white shadow-lg active:scale-95 transition-all">
                        {isVerifying ? <RefreshCw className="w-4 h-4 animate-spin" /> : <Shield className="w-4 h-4" />}
                        {isVerifying ? 'Verifying...' : 'Verify Shards'}
                    </button>
                    <button onClick={onForceRebalance} disabled={isRebalancing} className="flex items-center gap-2 px-5 py-2.5 rounded-xl font-bold bg-amber-600 hover:bg-amber-500 text-white shadow-lg active:scale-95 transition-all">
                        {isRebalancing ? <RefreshCw className="w-4 h-4 animate-spin" /> : <Shuffle className="w-4 h-4" />}
                        {isRebalancing ? 'Rebalancing...' : 'Force Rebalance'}
                    </button>
                    <button onClick={onRefreshStatus} aria-label="Refresh P2P backup status" className="p-2.5 bg-gray-700 hover:bg-gray-600 rounded-xl text-gray-300">
                        <RefreshCw className={`w-5 h-5 ${isLoadingStatus ? 'animate-spin' : ''}`} />
                    </button>
                </div>
            </div>

            <div className="p-8 space-y-6">
                {/* Stat chips row */}
                <div className="grid grid-cols-2 lg:grid-cols-4 gap-4">
                    <div className="bg-gray-900/50 rounded-xl p-4 border border-gray-700 flex items-center gap-4">
                        <div className="p-2.5 bg-green-900/30 rounded-lg"><Activity className="w-5 h-5 text-green-400" /></div>
                        <div>
                            <div className="text-[10px] text-gray-500 uppercase font-bold">Active Peers</div>
                            <div className="text-2xl font-black text-white">{p2pStatus?.active_peers ?? 0}</div>
                        </div>
                    </div>
                    <div className="bg-gray-900/50 rounded-xl p-4 border border-gray-700 flex items-center gap-4">
                        <div className="p-2.5 bg-indigo-900/30 rounded-lg"><Database className="w-5 h-5 text-indigo-400" /></div>
                        <div>
                            <div className="text-[10px] text-gray-500 uppercase font-bold">Total Shards</div>
                            <div className="text-2xl font-black text-indigo-400">{p2pStatus?.total_shards_stored ?? 0}</div>
                        </div>
                    </div>
                    <div className="bg-gray-900/50 rounded-xl p-4 border border-gray-700">
                        <div className="flex justify-between items-center text-xs mb-2">
                            <span className="text-gray-400 flex items-center gap-1.5"><Image className="w-3.5 h-3.5 text-blue-400" /> Photos</span>
                            <span className="flex items-center gap-2">
                                {p2pStatus && p2pStatus.pending_images > 0 && (
                                    <span className="text-[9px] text-amber-400 font-bold">{p2pStatus.pending_images.toLocaleString()} pending</span>
                                )}
                                <span className="text-white font-bold">{stats?.total_p2p_synced_images ?? 0} / {stats?.total_images ?? 0}</span>
                            </span>
                        </div>
                        <div className="h-2 bg-gray-800 rounded-full overflow-hidden">
                            <div className="h-full bg-blue-500 shadow-[0_0_8px_rgba(59,130,246,0.5)] transition-all duration-1000" style={{ width: `${stats?.total_images ? (stats.total_p2p_synced_images / stats.total_images) * 100 : 0}%` }} />
                        </div>
                    </div>
                    <div className="bg-gray-900/50 rounded-xl p-4 border border-gray-700">
                        <div className="flex justify-between items-center text-xs mb-2">
                            <span className="text-gray-400 flex items-center gap-1.5"><Video className="w-3.5 h-3.5 text-purple-400" /> Videos</span>
                            <span className="flex items-center gap-2">
                                {p2pStatus && p2pStatus.pending_videos > 0 && (
                                    <span className="text-[9px] text-amber-400 font-bold">{p2pStatus.pending_videos.toLocaleString()} pending</span>
                                )}
                                <span className="text-white font-bold">{stats?.total_p2p_synced_videos ?? 0} / {stats?.total_videos ?? 0}</span>
                            </span>
                        </div>
                        <div className="h-2 bg-gray-800 rounded-full overflow-hidden">
                            <div className="h-full bg-purple-500 shadow-[0_0_8px_rgba(168,85,247,0.5)] transition-all duration-1000" style={{ width: `${stats?.total_videos ? (stats.total_p2p_synced_videos / stats.total_videos) * 100 : 0}%` }} />
                        </div>
                    </div>
                </div>

                {/* Reconstruction health + DB backups */}
                <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
                    <div className="lg:col-span-2 bg-gray-900/50 rounded-2xl p-6 border border-gray-700">
                        <div className="flex flex-wrap items-center justify-between gap-3 mb-4">
                            <h3 className="text-xs font-bold text-gray-500 uppercase tracking-widest flex items-center gap-2">
                                <Shield className="w-4 h-4 text-teal-400" /> Reconstruction Health
                            </h3>
                            <span className={`px-3 py-1 rounded-full text-[10px] font-bold border ${(p2pStatus?.active_peers ?? 0) >= 5 ? 'bg-green-900/30 text-green-400 border-green-500/20' : (p2pStatus?.active_peers ?? 0) >= 3 ? 'bg-yellow-900/30 text-yellow-400 border-yellow-500/20' : 'bg-red-900/30 text-red-400 border-red-500/20'}`}>
                                {(p2pStatus?.active_peers ?? 0)} / 5 storage nodes
                            </span>
                        </div>
                        <p className="text-[10px] text-gray-500 mb-4">Needs 3 shards (nodes) to rebuild a file · 5 for full parity redundancy. Only shards on currently-reachable nodes count.</p>
                        <div className="grid grid-cols-2 lg:grid-cols-4 gap-4">
                            <div className="bg-gray-800/60 rounded-xl p-4 border border-emerald-500/20 text-center">
                                <div className="text-2xl font-black text-emerald-400">{p2pStatus?.ok_files?.toLocaleString() ?? 0}</div>
                                <div className="text-[9px] text-emerald-500/80 font-black uppercase mt-1">Full 5/5</div>
                            </div>
                            <div className="bg-gray-800/60 rounded-xl p-4 border border-yellow-500/20 text-center">
                                <div className="text-2xl font-black text-yellow-400">{p2pStatus?.degraded_files?.toLocaleString() ?? 0}</div>
                                <div className="text-[9px] text-yellow-500/80 font-black uppercase mt-1">Degraded 3-4/5</div>
                            </div>
                            <div className="bg-gray-800/60 rounded-xl p-4 border border-red-500/20 text-center">
                                <div className="text-2xl font-black text-red-400">{p2pStatus?.failed_files?.toLocaleString() ?? 0}</div>
                                <div className="text-[9px] text-red-500/80 font-black uppercase mt-1">At Risk 1-2/5</div>
                            </div>
                            <div className="bg-gray-800/60 rounded-xl p-4 border border-gray-700 text-center">
                                <div className="text-2xl font-black text-gray-400">{p2pStatus?.missing_files?.toLocaleString() ?? 0}</div>
                                <div className="text-[9px] text-gray-500 font-black uppercase mt-1">Offline 0/5</div>
                            </div>
                        </div>
                    </div>

                    <div className="bg-gray-900/50 rounded-2xl p-6 border border-gray-700">
                        <h3 className="text-xs font-bold text-gray-500 uppercase tracking-widest mb-4 flex items-center gap-2">
                            <Database className="w-4 h-4 text-indigo-400" /> Database Backups
                        </h3>
                        {p2pStatus && p2pStatus.db_backups_count > 0 ? (
                            <div className="space-y-4">
                                <div className="flex items-center justify-between">
                                    <span className="text-xs text-gray-500">Snapshots</span>
                                    <span className="text-xl font-black text-white">{p2pStatus.db_backups_count}</span>
                                </div>
                                <div className="flex items-center justify-between">
                                    <span className="text-xs text-gray-500">Total size</span>
                                    <span className="text-sm font-bold text-indigo-400">{formatBytes(p2pStatus.db_backups_total_bytes)}</span>
                                </div>
                                <div className="flex items-center justify-between">
                                    <span className="text-xs text-gray-500">Latest</span>
                                    <span className="text-sm font-bold text-gray-300">
                                        {p2pStatus.db_backups_latest_at ? new Date(p2pStatus.db_backups_latest_at).toLocaleString() : '—'}
                                    </span>
                                </div>
                                <p className="text-[10px] text-gray-600 pt-2 border-t border-gray-700/50">Rolling pg_dump snapshots distributed across the mesh via 3/5 erasure coding.</p>
                            </div>
                        ) : (
                            <p className="text-xs text-gray-500 italic">No database snapshots yet.</p>
                        )}
                    </div>
                </div>

                {/* Mesh Shard Distribution & Rebalance Progress */}
                <P2PRebalanceProgress />

                {/* Engine info + Peers */}
                <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
                    {/* Engine status */}
                    <div className="bg-gray-900/50 rounded-2xl p-6 border border-gray-700 flex flex-col gap-5">
                        <div className="flex justify-between items-start">
                            <div>
                                <h3 className="text-xs font-bold text-gray-500 uppercase tracking-widest flex items-center gap-2">
                                    <Server className="w-4 h-4 text-purple-400" /> P2P Engine
                                </h3>
                                <p className="text-lg font-bold text-white mt-1">Dynamic Mesh</p>
                            </div>
                            {p2pStatus?.health_status === 'critical'
                                ? <span className="px-3 py-1 bg-red-900/30 text-red-400 text-[10px] font-bold rounded-full border border-red-500/20">AT RISK</span>
                                : p2pStatus?.health_status === 'degraded'
                                ? <span className="px-3 py-1 bg-yellow-900/30 text-yellow-400 text-[10px] font-bold rounded-full border border-yellow-500/20">DEGRADED</span>
                                : <span className="px-3 py-1 bg-green-900/30 text-green-400 text-[10px] font-bold rounded-full border border-green-500/20">ONLINE</span>
                            }
                        </div>
                        <div className="pt-4 border-t border-gray-700/50 mt-auto">
                            <div className="text-[10px] text-gray-500 uppercase mb-2">Local Node ID</div>
                            <code className="text-[10px] text-blue-300 font-mono break-all bg-black/30 p-2.5 rounded-lg block">{p2pStatus?.local_peer_id || 'Generating...'}</code>
                        </div>
                    </div>

                    {/* Peers — spans 2 columns, uses 2-col internal grid */}
                    <div className="lg:col-span-2 bg-gray-900/50 rounded-2xl p-6 border border-gray-700">
                        <h3 className="text-xs font-bold text-gray-500 uppercase tracking-widest mb-4 flex items-center gap-2">
                            <Activity className="w-4 h-4 text-green-400" /> Storage Nodes
                        </h3>
                        {discoveredPeers.length > 0 ? (
                            <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
                                {discoveredPeers.map((peer) => (
                                    <div key={peer.peer_id} className={`border p-4 rounded-xl flex items-center justify-between group transition-colors ${peer.is_active ? 'bg-gray-800/80 border-gray-700/50 hover:border-blue-500/50' : 'bg-gray-900/40 border-gray-700/30 opacity-60'}`}>
                                        <div className="overflow-hidden min-w-0">
                                            <div className="text-[9px] text-gray-500 uppercase font-black">{peer.is_active ? 'Storage Node' : 'Offline Node'}</div>
                                            <code className="text-xs text-gray-300 font-mono block truncate">{peer.peer_id.substring(0, 20)}...</code>
                                            {peer.public_addr && (
                                                <div className="text-[9px] text-gray-600 font-mono mt-0.5 truncate">{peer.public_addr}</div>
                                            )}
                                            {peer.shard_count > 0 && (
                                                <div className={`text-[9px] font-bold mt-1 ${peer.is_active ? 'text-indigo-400' : 'text-yellow-600'}`}>
                                                    {peer.shard_count} shards{!peer.is_active && ' (pending rebalance)'}
                                                </div>
                                            )}
                                        </div>
                                        <div className="flex flex-col items-end gap-1 ml-3 flex-shrink-0">
                                            <span className={`w-2.5 h-2.5 rounded-full shadow-lg ${peer.is_active ? 'bg-green-400 animate-pulse shadow-green-500/20' : 'bg-red-500 shadow-red-500/20'}`} />
                                            <span className="text-[9px] text-gray-500">{new Date(peer.last_seen).toLocaleTimeString()}</span>
                                            {!peer.is_active && (
                                                <button
                                                    onClick={() => onRemoveNode(peer.peer_id, peer.shard_count)}
                                                    className="p-1.5 rounded-lg bg-red-900/40 hover:bg-red-600 text-red-400 hover:text-white transition-colors"
                                                    title="Remove offline node"
                                                    aria-label={`Remove offline node ${peer.peer_id.substring(0, 16)}`}
                                                >
                                                    <Trash2 className="w-3.5 h-3.5" />
                                                </button>
                                            )}
                                        </div>
                                    </div>
                                ))}
                            </div>
                        ) : (
                            <div className="text-center py-12 bg-gray-800/30 rounded-2xl border border-dashed border-gray-700">
                                <AlertTriangle className="w-8 h-8 text-gray-600 mx-auto mb-3" />
                                <p className="text-gray-500 text-xs font-medium">Waiting for peers...</p>
                            </div>
                        )}
                    </div>
                </div>
            </div>
        </div>
    );
}
