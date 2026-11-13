import { useEffect, useState } from "react";
import { observer } from "mobx-react-lite";
import { useStore } from "../stores/RootStore";
import { logger } from "../utils/logger";
import { Users, Cpu, Upload, Shield, TrendingUp, RefreshCw, Smartphone, Settings } from "lucide-react";
import { DirectoryImportModal } from "./DirectoryImportModal";
import { ServerImportModal } from "./ServerImportModal";
import { AndroidConnectionQR } from "./AndroidConnectionQR";
import { UserManagement } from "./UserManagement";
import { OverviewTab } from "./Dashboard/OverviewTab";
import { ImportTab } from "./Dashboard/ImportTab";
import { BackupTab } from "./Dashboard/BackupTab";
import { SystemTab } from "./Dashboard/SystemTab";
import { SettingsTab } from "./Dashboard/SettingsTab";
import { VerifyBackupModal } from "./Dashboard/VerifyBackupModal";

type DashboardTab = 'overview' | 'import' | 'system' | 'backup' | 'settings' | 'app' | 'users';

export const Dashboard = observer(() => {
    const { statsStore, authStore, uiStore, systemStore } = useStore();
    const isAdmin = authStore.user?.role === "admin";

    const [activeTab, setActiveTab] = useState<DashboardTab>('overview');

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
        if (isAdmin) statsStore.fetchAllStats();
    }, [statsStore, isAdmin]);

    // Pool stats are cheap and relevant on every admin tab, so they poll globally.
    useEffect(() => {
        if (!isAdmin) return;

        const tick = () => {
            if (!document.hidden) void statsStore.fetchPoolStats();
        };

        tick();
        const interval = setInterval(tick, 30000);
        document.addEventListener("visibilitychange", tick);

        return () => {
            clearInterval(interval);
            document.removeEventListener("visibilitychange", tick);
        };
    }, [statsStore, isAdmin]);

    // Heavy per-tab polling: logs/errors/alerts/gpu/models/pipeline/series belong
    // to the system tab and only load while it is active.
    useEffect(() => {
        if (!isAdmin) return;

        const loadForTab = () => {
            if (activeTab === 'system') {
                void statsStore.fetchSystemStats();
                void systemStore.loadLogs();
                void systemStore.loadErrors();
                void systemStore.loadAlerts();
                void systemStore.loadGpu();
                void systemStore.loadAiModels();
                void systemStore.loadPipeline();
                void systemStore.loadSeries();
            } else if (activeTab === 'overview') {
                void statsStore.fetchSystemStats();
            } else if (activeTab === 'backup') {
                const reb = statsStore.p2pBackupStatus?.rebalance;
                if (!(reb?.is_active || (reb && reb.progress_percent < 100))) {
                    void statsStore.fetchP2PBackupStatus();
                    void statsStore.fetchDiscoveredPeers();
                }
            }
        };

        loadForTab();
        const interval = setInterval(() => {
            if (!document.hidden) loadForTab();
        }, 30000);
        const onVisibilityChange = () => {
            if (!document.hidden) loadForTab();
        };
        document.addEventListener("visibilitychange", onVisibilityChange);

        return () => {
            clearInterval(interval);
            document.removeEventListener("visibilitychange", onVisibilityChange);
        };
    }, [statsStore, systemStore, isAdmin, activeTab]);

    // Adaptive fast-polling on the backup tab when rebalancing is active
    useEffect(() => {
        if (isAdmin && activeTab === 'backup') {
            void statsStore.fetchP2PBackupStatus();
            void statsStore.fetchDiscoveredPeers();

            const fastInterval = setInterval(() => {
                if (document.hidden !== false) return;
                const reb = statsStore.p2pBackupStatus?.rebalance;
                if (reb?.is_active || (reb && reb.progress_percent < 100)) {
                    void statsStore.fetchP2PBackupStatus();
                    void statsStore.fetchDiscoveredPeers();
                }
            }, 4000);
            return () => clearInterval(fastInterval);
        }
    }, [isAdmin, activeTab, statsStore]);

    // Verify modal: close on Escape
    useEffect(() => {
        if (!showVerifyModal) return;
        const handleKeyDown = (e: KeyboardEvent) => {
            if (e.key === "Escape") setShowVerifyModal(false);
        };
        document.addEventListener("keydown", handleKeyDown);
        return () => document.removeEventListener("keydown", handleKeyDown);
    }, [showVerifyModal]);

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
            logger.error("Failed to update settings:", error);
            uiStore.setError("Failed to update settings");
        }
    };

    const handleVerifyBackup = async () => {
        setIsVerifying(true);
        setShowVerifyModal(true);
        try {
            await statsStore.verifyP2PBackup();
        } catch {
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

    const tabClass = (tab: DashboardTab) => `px-6 py-3 text-sm font-medium transition-all duration-200 border-b-2 flex items-center gap-2 ${
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
                <button onClick={() => setActiveTab('users')} className={tabClass('users')}>
                    <Users size={18} /> Users
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
                        {activeTab === 'overview' && (
                            <OverviewTab stats={statsStore.stats} systemStats={statsStore.systemStats} />
                        )}
                        {activeTab === 'import' && (
                            <ImportTab
                                onOpenBrowserImport={() => setShowImportModal(true)}
                                onOpenServerImport={() => setShowServerImportModal(true)}
                            />
                        )}
                        {activeTab === 'backup' && (
                            <BackupTab
                                stats={statsStore.stats}
                                p2pStatus={statsStore.p2pBackupStatus}
                                discoveredPeers={statsStore.discoveredPeers || []}
                                isVerifying={isVerifying}
                                isRebalancing={isRebalancing}
                                isLoadingStatus={statsStore.isP2PBackupStatsLoading}
                                onVerifyBackup={handleVerifyBackup}
                                onForceRebalance={handleForceRebalance}
                                onRefreshStatus={() => statsStore.fetchP2PBackupStatus()}
                                onRemoveNode={handleRemoveNode}
                            />
                        )}
                        {activeTab === 'system' && (
                            <SystemTab
                                systemStats={statsStore.systemStats}
                                poolStats={statsStore.poolStats?.main_pool}
                                serviceHealth={statsStore.serviceHealth}
                                geoDbStats={statsStore.geoDbStats}
                                systemStore={systemStore}
                            />
                        )}
                        {activeTab === 'settings' && (
                            <SettingsTab
                                loaded={!!statsStore.aiSettings}
                                enableAiDescriptions={enableAiDescriptions}
                                setEnableAiDescriptions={setEnableAiDescriptions}
                                enableEmbeddings={enableEmbeddings}
                                setEnableEmbeddings={setEnableEmbeddings}
                                enableFaceDetection={enableFaceDetection}
                                setEnableFaceDetection={setEnableFaceDetection}
                                onSave={handleUpdateSettings}
                            />
                        )}
                        {activeTab === 'app' && <AndroidConnectionQR />}
                        {activeTab === 'users' && <UserManagement />}
                    </>
                )}
            </div>

            {/* Modals */}
            {showImportModal && <DirectoryImportModal onClose={() => setShowImportModal(false)} />}
            {showServerImportModal && <ServerImportModal onClose={() => setShowServerImportModal(false)} />}
            {showVerifyModal && (
                <VerifyBackupModal
                    isVerifying={isVerifying}
                    verificationResult={statsStore.verificationResult}
                    onClose={() => setShowVerifyModal(false)}
                    onStartVerification={handleVerifyBackup}
                />
            )}
        </div>
    );
});
