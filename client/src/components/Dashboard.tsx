import { useEffect, useState } from "react";
import { observer } from "mobx-react-lite";
import { useStore } from "../stores/RootStore";
import { Image, Video, Users, CheckCircle, FileText, Database, Activity, Settings, Brain, Cpu, HardDrive, MemoryStick, Zap, Upload, Folder, Network, Shield, TrendingUp, RefreshCw, AlertTriangle, Server, Clock, X, Shuffle, Trash2, Smartphone } from "lucide-react";
import { DirectoryImportModal } from "./DirectoryImportModal";
import { ServerImportModal } from "./ServerImportModal";
import { AndroidConnectionQR } from "./AndroidConnectionQR";

export const Dashboard = observer(() => {
    const { statsStore, authStore } = useStore();
    const isAdmin = authStore.user?.role === "admin";

    const [activeTab, setActiveTab] = useState<'overview' | 'import' | 'system' | 'backup' | 'settings' | 'app'>('overview');

    // Local state for settings form
    const [enableAiDescriptions, setEnableAiDescriptions] = useState<boolean>(true);
    const [enableEmbeddings, setEnableEmbeddings] = useState<boolean>(true);
    const [embeddingParallelCount, setEmbeddingParallelCount] = useState<number>(10);
    const [enableFaceDetection, setEnableFaceDetection] = useState<boolean>(true);
    const [faceDetectionParallelCount, setFaceDetectionParallelCount] = useState<number>(3);
    const [enableMediaBackup, setEnableMediaBackup] = useState<boolean>(false);
    
    const [showImportModal, setShowImportModal] = useState<boolean>(false);
    const [showServerImportModal, setShowServerImportModal] = useState<boolean>(false);
    const [showVerifyModal, setShowVerifyModal] = useState<boolean>(false);
    const [isVerifying, setIsVerifying] = useState<boolean>(false);
    const [isRebalancing, setIsRebalancing] = useState<boolean>(false);

    useEffect(() => {
        if (isAdmin) {
            statsStore.fetchAllStats();

            const interval = setInterval(() => {
                statsStore.fetchPoolStats();
                statsStore.fetchSystemStats();
                statsStore.fetchP2PBackupStatus();
                statsStore.fetchDiscoveredPeers();
            }, 30000);

            return () => clearInterval(interval);
        }
    }, [statsStore, isAdmin]);

    // Update local state when settings are loaded
    useEffect(() => {
        if (statsStore.aiSettings) {
            setEnableAiDescriptions(statsStore.aiSettings.enable_ai_descriptions);
            setEnableEmbeddings(statsStore.aiSettings.enable_embeddings);
            setEmbeddingParallelCount(statsStore.aiSettings.embedding_parallel_count);
            setEnableFaceDetection(statsStore.aiSettings.enable_face_detection);
            setFaceDetectionParallelCount(statsStore.aiSettings.face_detection_parallel_count);
            setEnableMediaBackup(statsStore.aiSettings.enable_media_backup);
        }
    }, [statsStore.aiSettings]);

    if (!isAdmin) {
        return (
            <div className="p-8 text-center bg-gray-800 rounded-lg border border-gray-700 mt-4">
                <Users className="w-12 h-12 text-gray-500 mx-auto mb-4" />
                <h2 className="text-xl font-bold text-gray-100">Access Restricted</h2>
                <p className="text-gray-400 mt-2">Only administrators can view system statistics and backups.</p>
            </div>
        );
    }

    const stats = statsStore.stats;
    const poolStats = statsStore.poolStats?.main_pool;
    const systemStats = statsStore.systemStats;
    const p2pStatus = statsStore.p2pBackupStatus;
    const discoveredPeers = statsStore.discoveredPeers || [];

    const handleUpdateSettings = async () => {
        try {
            await statsStore.updateAiSettings({
                enable_ai_descriptions: enableAiDescriptions,
                enable_embeddings: enableEmbeddings,
                embedding_parallel_count: embeddingParallelCount,
                enable_face_detection: enableFaceDetection,
                face_detection_parallel_count: faceDetectionParallelCount,
                enable_media_backup: enableMediaBackup,
            });
        } catch (error) {
            console.error("Failed to update settings:", error);
        }
    };

    const handleVerifyBackup = async () => {
        setIsVerifying(true);
        setShowVerifyModal(true);
        try {
            await statsStore.verifyP2PBackup();
        } catch (error) {
            // Error handled in store
        } finally {
            setIsVerifying(false);
        }
    };

    const handleForceRebalance = async () => {
        setIsRebalancing(true);
        try { await statsStore.forceRebalance(); }
        finally { setIsRebalancing(false); }
    };

    const handleRemoveNode = async (peerId: string, shardCount: number) => {
        if (!window.confirm(`Remove offline node ${peerId.substring(0, 16)}... and delete its ${shardCount} shards?`)) return;
        try {
            await statsStore.removeNode(peerId);
        } catch {
            // error toast shown by store
        }
    };

    const statCards = [
        { title: "Total Images", value: stats?.total_images, icon: <Image className="w-8 h-8 text-blue-500" /> },
        { title: "Total Videos", value: stats?.total_videos, icon: <Video className="w-8 h-8 text-purple-500" /> },
        { title: "Total Users", value: stats?.total_users, icon: <Users className="w-8 h-8 text-green-500" /> },
        { title: "Images with Description", value: stats?.images_with_description, icon: <FileText className="w-8 h-8 text-yellow-500" /> },
        { title: "Images with Embedding", value: stats?.images_with_embedding, icon: <Brain className="w-8 h-8 text-indigo-500" /> },
        { title: "Starred Images", value: stats?.starred_images, icon: <Image className="w-8 h-8 text-red-500" /> },
        { title: "Verified Images", value: stats?.verified_images, icon: <CheckCircle className="w-8 h-8 text-pink-500" /> },
        { title: "Total Faces", value: stats?.total_faces, icon: <Users className="w-8 h-8 text-cyan-500" /> },
        { title: "Total People", value: stats?.total_persons, icon: <Users className="w-8 h-8 text-orange-500" /> },
        { title: "Disk Available", value: systemStats?.disk_available_gb !== undefined ? `${systemStats.disk_available_gb.toFixed(1)} GB` : '...', icon: <HardDrive className="w-8 h-8 text-emerald-500" /> },
    ];

    const tabClass = (tab: string) => `px-6 py-3 text-sm font-medium transition-all duration-200 border-b-2 flex items-center gap-2 ${
        activeTab === tab 
        ? 'border-blue-500 text-blue-400 bg-gray-800/50' 
        : 'border-transparent text-gray-400 hover:text-gray-200 hover:bg-gray-800/30'
    }`;

    return (
        <div className="animate-in fade-in duration-500">
            {/* Tab Navigation */}
            <div className="flex border-b border-gray-700 mb-8 overflow-x-auto scrollbar-hide">
                <button onClick={() => setActiveTab('overview')} className={tabClass('overview')}>
                    <TrendingUp size={18} /> Overview
                </button>
                <button onClick={() => setActiveTab('import')} className={tabClass('import')}>
                    <Upload size={18} /> Import
                </button>
                <button onClick={() => setActiveTab('backup')} className={tabClass('backup')}>
                    <Shield size={18} /> Backup
                </button>
                <button onClick={() => setActiveTab('system')} className={tabClass('system')}>
                    <Cpu size={18} /> System
                </button>
                <button onClick={() => setActiveTab('settings')} className={tabClass('settings')}>
                    <Settings size={18} /> Settings
                </button>
                <button onClick={() => setActiveTab('app')} className={tabClass('app')}>
                    <Smartphone size={18} /> App Setup
                </button>
            </div>

            {/* --- Main Content Area --- */}
            <div className="min-h-[400px]">
                {statsStore.isLoading && activeTab === 'overview' ? (
                    <div className="flex flex-col items-center justify-center py-20 text-gray-400">
                        <RefreshCw className="w-10 h-10 animate-spin mb-4 text-blue-500" />
                        <p>Loading your memory vault stats...</p>
                    </div>
                ) : (
                    <>
                        {/* --- Overview Tab --- */}
                        {activeTab === 'overview' && (
                            <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-6">
                                {statCards.map((card, index) => (
                                    <div key={index} className="bg-gray-800 shadow-lg rounded-xl border border-gray-700 p-5 flex items-center transition-transform hover:scale-[1.02]">
                                        <div className="p-3 bg-gray-900/50 rounded-lg">{card.icon}</div>
                                        <div className="ml-5">
                                            <dt className="text-xs font-medium text-gray-500 uppercase tracking-wider">{card.title}</dt>
                                            <dd className="text-2xl font-bold text-gray-100">{card.value?.toLocaleString() ?? '...'}</dd>
                                        </div>
                                    </div>
                                ))}
                            </div>
                        )}

                        {/* --- Import Tab --- */}
                        {activeTab === 'import' && (
                            <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
                                <div className="bg-gray-800 shadow-lg rounded-xl p-8 border border-gray-700">
                                    <div className="flex items-center gap-4 mb-6">
                                        <div className="p-3 bg-blue-900/30 rounded-lg"><Upload className="w-8 h-8 text-blue-400" /></div>
                                        <div>
                                            <h2 className="text-xl font-bold text-gray-100">Browser Upload</h2>
                                            <p className="text-gray-400 text-sm">Upload directly from this device.</p>
                                        </div>
                                    </div>
                                    <button onClick={() => setShowImportModal(true)} className="w-full py-4 bg-blue-600 hover:bg-blue-700 text-white font-bold rounded-lg shadow-lg active:scale-[0.98] transition-all">
                                        Select Files
                                    </button>
                                </div>
                                <div className="bg-gray-800 shadow-lg rounded-xl p-8 border border-gray-700">
                                    <div className="flex items-center gap-4 mb-6">
                                        <div className="p-3 bg-indigo-900/30 rounded-lg"><Folder className="w-8 h-8 text-indigo-400" /></div>
                                        <div>
                                            <h2 className="text-xl font-bold text-gray-100">Server Import</h2>
                                            <p className="text-gray-400 text-sm">Scan a folder on the server's disk.</p>
                                        </div>
                                    </div>
                                    <button onClick={() => setShowServerImportModal(true)} className="w-full py-4 bg-indigo-600 hover:bg-indigo-700 text-white font-bold rounded-lg shadow-lg active:scale-[0.98] transition-all">
                                        Choose Directory
                                    </button>
                                </div>
                            </div>
                        )}

                        {/* --- Backup Tab --- */}
                        {activeTab === 'backup' && (
                            <div className="bg-gray-800/80 backdrop-blur-md shadow-2xl rounded-2xl border border-gray-700 overflow-hidden">
                                <div className="px-8 py-6 border-b border-gray-700 bg-gray-800/50 flex justify-between items-center">
                                    <div>
                                        <h2 className="text-2xl font-bold text-gray-100 flex items-center">
                                            <Network className="w-7 h-7 text-blue-400 mr-3" /> P2P Backup
                                        </h2>
                                        <p className="text-gray-500 text-sm mt-1">Distributed 3/5 Reed-Solomon Sharding</p>
                                    </div>
                                    <div className="flex items-center gap-3">
                                        <button onClick={handleVerifyBackup} disabled={isVerifying} className="flex items-center gap-2 px-5 py-2.5 rounded-xl font-bold bg-emerald-600 hover:bg-emerald-500 text-white shadow-lg active:scale-95 transition-all">
                                            {isVerifying ? <RefreshCw className="w-4 h-4 animate-spin" /> : <Shield className="w-4 h-4" />}
                                            {isVerifying ? 'Verifying...' : 'Verify Shards'}
                                        </button>
                                        <button onClick={handleForceRebalance} disabled={isRebalancing} className="flex items-center gap-2 px-5 py-2.5 rounded-xl font-bold bg-amber-600 hover:bg-amber-500 text-white shadow-lg active:scale-95 transition-all">
                                            {isRebalancing ? <RefreshCw className="w-4 h-4 animate-spin" /> : <Shuffle className="w-4 h-4" />}
                                            {isRebalancing ? 'Rebalancing...' : 'Force Rebalance'}
                                        </button>
                                        <button onClick={() => statsStore.fetchP2PBackupStatus()} className="p-2.5 bg-gray-700 hover:bg-gray-600 rounded-xl text-gray-300">
                                            <RefreshCw className={`w-5 h-5 ${statsStore.isP2PBackupStatsLoading ? 'animate-spin' : ''}`} />
                                        </button>
                                    </div>
                                </div>

                                <div className="p-8">
                                    <div className="grid grid-cols-1 lg:grid-cols-3 gap-8">
                                        <div className="space-y-6">
                                            <div className="bg-gray-900/50 rounded-2xl p-6 border border-gray-700">
                                                <div className="flex justify-between items-start mb-6">
                                                    <div>
                                                        <h3 className="text-xs font-bold text-gray-500 uppercase tracking-widest flex items-center gap-2">
                                                            <Server className="w-4 h-4 text-purple-400" /> P2P Engine
                                                        </h3>
                                                        <p className="text-lg font-bold text-white mt-1">Static Overlay</p>
                                                    </div>
                                                    <span className="px-3 py-1 bg-green-900/30 text-green-400 text-[10px] font-bold rounded-full border border-green-500/20">ONLINE</span>
                                                </div>
                                                <div className="grid grid-cols-2 gap-4 mb-6">
                                                    <div className="p-3 bg-gray-800 rounded-xl border border-gray-700/50">
                                                        <div className="text-[10px] text-gray-500 uppercase">Configured Peers</div>
                                                        <div className="text-xl font-black text-white">
                                                            {statsStore.p2pDaemonStatus?.p2p_peer_count ?? 0}
                                                        </div>
                                                    </div>
                                                    <div className="p-3 bg-gray-800 rounded-xl border border-gray-700/50">
                                                        <div className="text-[10px] text-gray-500 uppercase">Shards</div>
                                                        <div className="text-xl font-black text-indigo-400">{p2pStatus?.total_shards_stored ?? 0}</div>
                                                    </div>
                                                </div>
                                                <div className="pt-4 border-t border-gray-700/50">
                                                    <div className="text-[10px] text-gray-500 uppercase mb-2">Local Node ID</div>
                                                    <code className="text-[10px] text-blue-300 font-mono break-all bg-black/30 p-2 rounded block">{p2pStatus?.local_peer_id || 'Generating...'}</code>
                                                </div>
                                            </div>

                                            {/* Replication Progress */}
                                            <div className="bg-gray-900/50 rounded-2xl p-6 border border-gray-700">
                                                <h3 className="text-xs font-bold text-gray-500 uppercase tracking-widest mb-6 flex items-center gap-2">
                                                    <Image className="w-4 h-4 text-indigo-400" /> Replication
                                                </h3>
                                                <div className="space-y-6">
                                                    <div>
                                                        <div className="flex justify-between text-xs mb-2">
                                                            <span className="text-gray-400">Photos Synced</span>
                                                            <span className="text-white font-bold">{stats?.total_p2p_synced_images ?? 0} / {stats?.total_images ?? 0}</span>
                                                        </div>
                                                        <div className="h-2 bg-gray-800 rounded-full overflow-hidden">
                                                            <div className="h-full bg-blue-500 shadow-[0_0_10px_rgba(59,130,246,0.5)] transition-all duration-1000" style={{ width: `${stats?.total_images ? (stats.total_p2p_synced_images / stats.total_images) * 100 : 0}%` }}></div>
                                                        </div>
                                                    </div>
                                                    <div>
                                                        <div className="flex justify-between text-xs mb-2">
                                                            <span className="text-gray-400">Videos Synced</span>
                                                            <span className="text-white font-bold">{stats?.total_p2p_synced_videos ?? 0} / {stats?.total_videos ?? 0}</span>
                                                        </div>
                                                        <div className="h-2 bg-gray-800 rounded-full overflow-hidden">
                                                            <div className="h-full bg-purple-500 shadow-[0_0_10px_rgba(168,85,247,0.5)] transition-all duration-1000" style={{ width: `${stats?.total_videos ? (stats.total_p2p_synced_videos / stats.total_videos) * 100 : 0}%` }}></div>
                                                        </div>
                                                    </div>
                                                </div>
                                            </div>
                                        </div>

                                        {/* Peer List */}
                                        <div className="bg-gray-900/50 rounded-2xl p-6 border border-gray-700 flex flex-col">
                                            <h3 className="text-xs font-bold text-gray-500 uppercase tracking-widest mb-6 flex items-center gap-2">
                                                <Activity className="w-4 h-4 text-green-400" /> Active Peers
                                            </h3>
                                            <div className="flex-1 space-y-3 overflow-y-auto pr-2 scrollbar-thin scrollbar-thumb-gray-700">
                                                {discoveredPeers.length > 0 ? discoveredPeers.map((peer) => (
                                                    <div key={peer.peer_id} className={`border p-4 rounded-xl flex items-center justify-between group transition-colors ${peer.is_active ? 'bg-gray-800/80 border-gray-700/50 hover:border-blue-500/50' : 'bg-gray-900/40 border-gray-700/30 opacity-60'}`}>
                                                        <div className="overflow-hidden">
                                                            <div className="text-[9px] text-gray-500 uppercase font-black">{peer.is_active ? 'Storage Node' : 'Offline Node'}</div>
                                                            <code className="text-xs text-gray-300 font-mono block truncate">{peer.peer_id.substring(0, 16)}...</code>
                                                            {peer.shard_count > 0 && (
                                                                <div className={`text-[9px] font-bold mt-1 ${peer.is_active ? 'text-indigo-400' : 'text-yellow-600'}`}>
                                                                    {peer.shard_count} shards{!peer.is_active && ' (pending rebalance)'}
                                                                </div>
                                                            )}
                                                        </div>
                                                        <div className="flex flex-col items-end gap-1">
                                                            <span className={`w-2.5 h-2.5 rounded-full shadow-lg ${peer.is_active ? 'bg-green-400 animate-pulse shadow-green-500/20' : 'bg-red-500 shadow-red-500/20'}`}></span>
                                                            <span className="text-[9px] text-gray-500">{new Date(peer.last_seen).toLocaleTimeString()}</span>
                                                            {!peer.is_active && (
                                                                <button onClick={() => handleRemoveNode(peer.peer_id, peer.shard_count)}
                                                                    className="p-1.5 rounded-lg bg-red-900/40 hover:bg-red-600 text-red-400 hover:text-white transition-colors"
                                                                    title="Remove offline node">
                                                                    <Trash2 className="w-3.5 h-3.5" />
                                                                </button>
                                                            )}
                                                        </div>
                                                    </div>
                                                )) : (
                                                    <div className="text-center py-12 bg-gray-800/30 rounded-2xl border border-dashed border-gray-700">
                                                        <AlertTriangle className="w-8 h-8 text-gray-600 mx-auto mb-3" />
                                                        <p className="text-gray-500 text-xs font-medium">Waiting for peers...</p>
                                                    </div>
                                                )}
                                            </div>
                                        </div>

                                    </div>
                                </div>
                            </div>
                        )}

                        {/* --- System Tab --- */}
                        {activeTab === 'system' && (
                            <div className="space-y-6">
                                <div className="bg-gray-800 shadow-xl rounded-xl p-8 border border-gray-700">
                                    <h2 className="text-xl font-bold text-gray-100 mb-8 flex items-center"><Activity className="w-6 h-6 text-blue-400 mr-3" /> System Resources</h2>
                                    {systemStats ? (
                                        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-8">
                                            <div className="p-4 bg-gray-900/50 rounded-xl border border-gray-700">
                                                <dt className="text-xs font-black text-gray-500 uppercase mb-3 flex items-center gap-2"><Cpu className="w-4 h-4 text-blue-400" /> CPU</dt>
                                                <dd className="text-3xl font-black text-white">{systemStats.cpu_usage_percent.toFixed(1)}%</dd>
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
                                            </div>
                                            <div className="p-4 bg-gray-900/50 rounded-xl border border-gray-700">
                                                <dt className="text-xs font-black text-gray-500 uppercase mb-3 flex items-center gap-2"><Clock className="w-4 h-4 text-orange-400" /> Uptime</dt>
                                                <dd className="text-2xl font-black text-white">{Math.floor(systemStats.uptime_seconds / 3600)}h {Math.floor((systemStats.uptime_seconds % 3600) / 60)}m</dd>
                                            </div>
                                        </div>
                                    ) : <div className="text-gray-500 italic">Waiting for system telemetry...</div>}
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
                            </div>
                        )}

                        {/* --- Settings Tab --- */}
                        {activeTab === 'settings' && (
                            <div className="bg-gray-800 shadow-xl rounded-xl p-8 border border-gray-700">
                                <h2 className="text-xl font-bold text-gray-100 mb-8 flex items-center"><Brain className="w-6 h-6 text-pink-400 mr-3" /> AI & Processing</h2>
                                {statsStore.aiSettings ? (
                                    <div className="space-y-10">
                                        <div className="grid grid-cols-1 md:grid-cols-3 gap-8">
                                            <label className="flex items-start cursor-pointer p-4 bg-gray-900/30 rounded-xl border border-gray-700 hover:border-blue-500/50 transition-colors">
                                                <input type="checkbox" checked={enableAiDescriptions} onChange={(e) => setEnableAiDescriptions(e.target.checked)} className="w-6 h-6 mt-1 text-blue-600 bg-gray-700 border-gray-600 rounded focus:ring-blue-500" />
                                                <div className="ml-4">
                                                    <span className="text-gray-200 font-bold block">AI Captions</span>
                                                    <p className="text-[10px] text-gray-500 mt-1 uppercase font-black">Automatic descriptions for images</p>
                                                </div>
                                            </label>
                                            <label className="flex items-start cursor-pointer p-4 bg-gray-900/30 rounded-xl border border-gray-700 hover:border-purple-500/50 transition-colors">
                                                <input type="checkbox" checked={enableEmbeddings} onChange={(e) => setEnableEmbeddings(e.target.checked)} className="w-6 h-6 mt-1 text-purple-600 bg-gray-700 border-gray-600 rounded focus:ring-purple-500" />
                                                <div className="ml-4">
                                                    <span className="text-gray-200 font-bold block">Semantic Index</span>
                                                    <p className="text-[10px] text-gray-500 mt-1 uppercase font-black">Enable natural language search</p>
                                                </div>
                                            </label>
                                            <label className="flex items-start cursor-pointer p-4 bg-gray-900/30 rounded-xl border border-gray-700 hover:border-orange-500/50 transition-colors">
                                                <input type="checkbox" checked={enableFaceDetection} onChange={(e) => setEnableFaceDetection(e.target.checked)} className="w-6 h-6 mt-1 text-orange-600 bg-gray-700 border-gray-600 rounded focus:ring-orange-500" />
                                                <div className="ml-4">
                                                    <span className="text-gray-200 font-bold block">Face Grouping</span>
                                                    <p className="text-[10px] text-gray-500 mt-1 uppercase font-black">Identify and cluster people</p>
                                                </div>
                                            </label>
                                        </div>
                                        <div className="flex justify-end pt-6 border-t border-gray-700">
                                            <button onClick={handleUpdateSettings} className="px-10 py-4 bg-blue-600 hover:bg-blue-700 text-white font-black rounded-xl shadow-xl shadow-blue-500/20 transition-all active:scale-95">
                                                SAVE PREFERENCES
                                            </button>
                                        </div>
                                    </div>
                                ) : <div className="text-gray-500 italic">Syncing settings from vault...</div>}
                            </div>
                        )}

                        {/* --- App Setup Tab --- */}
                        {activeTab === 'app' && (
                            <div className="max-w-sm">
                                <AndroidConnectionQR />
                            </div>
                        )}
                    </>
                )}
            </div>

            {/* Modals */}
            {showImportModal && <DirectoryImportModal onClose={() => setShowImportModal(false)} />}
            {showServerImportModal && <ServerImportModal onClose={() => setShowServerImportModal(false)} />}

            {showVerifyModal && (
                <div className="fixed inset-0 bg-black/80 backdrop-blur-xl flex items-center justify-center z-50 p-4">
                    <div className="bg-gray-800 rounded-3xl border border-gray-700 shadow-2xl max-w-2xl w-full overflow-hidden">
                        <div className="px-8 py-6 border-b border-gray-700 flex justify-between items-center bg-gray-800/50">
                            <h3 className="text-xl font-black text-gray-100 flex items-center gap-3"><Shield className="w-6 h-6 text-emerald-400" /> SHARD VERIFICATION</h3>
                            <button onClick={() => setShowVerifyModal(false)} className="p-2 hover:bg-gray-700 rounded-full transition-colors"><X className="w-6 h-6 text-gray-400" /></button>
                        </div>
                        <div className="p-8">
                            {isVerifying ? (
                                <div className="flex flex-col items-center justify-center py-16">
                                    <RefreshCw className="w-16 h-16 text-emerald-400 animate-spin mb-6" />
                                    <p className="text-gray-100 text-2xl font-black tracking-tight">Checking Integrity...</p>
                                    <p className="text-gray-500 mt-2 font-bold uppercase text-xs tracking-widest">Scanning network shards</p>
                                </div>
                            ) : statsStore.verificationResult ? (
                                <div className="space-y-8">
                                    <div className="grid grid-cols-3 gap-6">
                                        <div className="bg-emerald-900/20 border border-emerald-500/20 rounded-2xl p-6 text-center">
                                            <div className="text-3xl font-black text-emerald-400">{statsStore.verificationResult.verified_files}</div>
                                            <div className="text-[10px] text-emerald-500 font-black uppercase mt-1">Healthy</div>
                                        </div>
                                        <div className="bg-red-900/20 border border-red-500/20 rounded-2xl p-6 text-center">
                                            <div className="text-3xl font-black text-red-400">{statsStore.verificationResult.failed_files}</div>
                                            <div className="text-[10px] text-red-500 font-black uppercase mt-1">Failed</div>
                                        </div>
                                        <div className="bg-yellow-900/20 border border-yellow-500/20 rounded-2xl p-6 text-center">
                                            <div className="text-3xl font-black text-yellow-400">{statsStore.verificationResult.missing_files}</div>
                                            <div className="text-[10px] text-yellow-500 font-black uppercase mt-1">Missing</div>
                                        </div>
                                    </div>
                                    <div className="space-y-2 max-h-[300px] overflow-y-auto pr-2 scrollbar-thin">
                                        {statsStore.verificationResult.files.map((file, idx) => {
                                            const statusColor = file.status === 'ok'
                                                ? 'text-emerald-400 border-emerald-500/20 bg-emerald-500/5'
                                                : file.status === 'degraded'
                                                ? 'text-yellow-400 border-yellow-500/20 bg-yellow-500/5'
                                                : file.status === 'failed'
                                                ? 'text-red-400 border-red-500/20 bg-red-500/5'
                                                : 'text-gray-500 border-gray-500/20 bg-gray-500/5';
                                            return (
                                                <div key={idx} className="p-3 rounded-xl border bg-gray-900/50 border-gray-700/50 flex items-center justify-between gap-3 group">
                                                    <code className="text-[10px] text-gray-500 font-mono group-hover:text-blue-400 transition-colors truncate">{file.root_hash.substring(0, 28)}...</code>
                                                    <div className="flex items-center gap-2 shrink-0">
                                                        <span className="text-[9px] text-gray-600 font-mono">{file.shards_available}/{file.shards_total}</span>
                                                        <div className={`text-[10px] font-black px-2 py-0.5 rounded-full border ${statusColor}`}>
                                                            {file.status.toUpperCase()}
                                                        </div>
                                                    </div>
                                                </div>
                                            );
                                        })}
                                    </div>
                                </div>
                            ) : (
                                <div className="text-center py-16">
                                    <Shield className="w-16 h-16 text-gray-700 mx-auto mb-6" />
                                    <p className="text-gray-400 font-bold uppercase text-xs tracking-widest">Click below to start deep scan</p>
                                    <button onClick={handleVerifyBackup} className="mt-8 px-10 py-4 bg-emerald-600 hover:bg-emerald-500 text-white font-black rounded-2xl shadow-2xl transition-all active:scale-95">
                                        START VERIFICATION
                                    </button>
                                </div>
                            )}
                        </div>
                        <div className="px-8 py-6 border-t border-gray-700 bg-gray-800/50 flex justify-end">
                            <button onClick={() => setShowVerifyModal(false)} className="px-8 py-3 bg-gray-700 hover:bg-gray-600 text-gray-200 font-bold rounded-xl transition-all">Close</button>
                        </div>
                    </div>
                </div>
            )}
        </div>
    );
});
