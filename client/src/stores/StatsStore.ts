import { makeAutoObservable, runInAction } from "mobx";
import axios from "../api/axiosConfig";
import { RootStore } from "./RootStore";

export interface DashboardStats {
    total_images: number;
    total_videos: number;
    total_users: number;
    images_with_description: number;
    starred_images: number;
    starred_videos: number;
    images_with_embedding: number;
    verified_images: number;
    verified_videos: number;
    total_faces: number;
    total_persons: number;
    images_with_faces: number;
    total_p2p_synced_images: number;
    total_p2p_synced_videos: number;
    thumbnail_count: number;
}

export interface PoolMetrics {
    size: number;
    available: number;
    max_size: number;
    utilization_percent: number;
}

export interface PoolStats {
    main_pool: PoolMetrics;
}

export interface GeoDbStats {
    total_boundaries: number;
    countries: number;
    states_provinces: number;
    counties: number;
    cities: number;
    other_boundaries: number;
    unique_countries: number;
}

export interface AiSettings {
    enable_ai_descriptions: boolean;
    enable_embeddings: boolean;
    embedding_parallel_count: number;
    enable_face_detection: boolean;
    face_detection_parallel_count: number;
    enable_media_backup: boolean;
}

export interface UpdateAiSettingsRequest {
    enable_ai_descriptions?: boolean;
    enable_embeddings?: boolean;
    embedding_parallel_count?: number;
    enable_face_detection?: boolean;
    face_detection_parallel_count?: number;
    enable_media_backup?: boolean;
}

export interface SystemStats {
    cpu_usage_percent: number;
    memory_used_gb: number;
    memory_total_gb: number;
    memory_usage_percent: number;
    disk_used_gb: number;
    disk_total_gb: number;
    disk_available_gb: number;
    disk_usage_percent: number;
    gpu_info: string | null;
    gpu_usage_percent: number | null;
    gpu_memory_used_mb: number | null;
    gpu_memory_total_mb: number | null;
    uptime_seconds: number;
}

export interface P2PDaemonStatus {
    is_healthy: boolean;
    node_id: string;
    active_peers: number;
    blobs_stored: number;
    bytes_stored: number;
    bytes_uploaded: number;
    files_uploaded: number;
    p2p_peer_count: number;
}

export interface P2PBackupStatus {
    local_peer_id: string;
    is_healthy: boolean;
    active_peers: number;
    total_shards_stored: number;
}

export interface DiscoveredPeer {
    peer_id: string;
    last_seen: string;
    is_active: boolean;
    shard_count: number;
}

export interface DiscoveredPeersResponse {
    peer_count: number;
    peers: DiscoveredPeer[];
}

export interface FileVerifyResult {
    root_hash: string;
    status: string;
    shards_available: number;
    shards_required: number;
    shards_total: number;
    error: string | null;
}

export interface VerificationResult {
    total_files: number;
    verified_files: number;
    failed_files: number;
    missing_files: number;
    files: FileVerifyResult[];
}

export class StatsStore {
    rootStore: RootStore;
    stats: DashboardStats | null = null;
    poolStats: PoolStats | null = null;
    geoDbStats: GeoDbStats | null = null;
    aiSettings: AiSettings | null = null;
    systemStats: SystemStats | null = null;
    p2pBackupStatus: P2PBackupStatus | null = null;
    p2pDaemonStatus: P2PDaemonStatus | null = null;
    discoveredPeers: DiscoveredPeer[] = [];
    verificationResult: VerificationResult | null = null;
    isLoading: boolean = false;
    isPoolStatsLoading: boolean = false;
    isGeoDbStatsLoading: boolean = false;
    isAiSettingsLoading: boolean = false;
    isSystemStatsLoading: boolean = false;
    isP2PBackupStatsLoading: boolean = false;
    isP2PDaemonStatusLoading: boolean = false;
    isDiscoveredPeersLoading: boolean = false;

    constructor(rootStore: RootStore) {
        makeAutoObservable(this);
        this.rootStore = rootStore;
    }

    fetchStats = async () => {
        this.isLoading = true;
        try {
            const response = await axios.get<DashboardStats>("/stats");
            runInAction(() => {
                this.stats = response.data;
            });
        } catch (error) {
            console.error("Failed to fetch dashboard stats", error);
            this.rootStore.uiStore.setError("Failed to load dashboard statistics.");
        } finally {
            runInAction(() => {
                this.isLoading = false;
            });
        }
    };

    fetchPoolStats = async () => {
        this.isPoolStatsLoading = true;
        try {
            const response = await axios.get<PoolStats>("/pool-stats");
            runInAction(() => {
                this.poolStats = response.data;
            });
        } catch (error) {
            console.error("Failed to fetch pool stats", error);
        } finally {
            runInAction(() => {
                this.isPoolStatsLoading = false;
            });
        }
    };

    fetchGeoDbStats = async () => {
        this.isGeoDbStatsLoading = true;
        try {
            const response = await axios.get<GeoDbStats>("/geodb-stats");
            runInAction(() => {
                this.geoDbStats = response.data;
            });
        } catch (error) {
            console.error("Failed to fetch geodb stats", error);
        } finally {
            runInAction(() => {
                this.isGeoDbStatsLoading = false;
            });
        }
    };

    fetchAiSettings = async () => {
        this.isAiSettingsLoading = true;
        try {
            const response = await axios.get<AiSettings>("/ai-settings");
            runInAction(() => {
                this.aiSettings = response.data;
            });
        } catch (error) {
            console.error("Failed to fetch AI settings", error);
        } finally {
            runInAction(() => {
                this.isAiSettingsLoading = false;
            });
        }
    };

    updateAiSettings = async (request: UpdateAiSettingsRequest) => {
        try {
            const response = await axios.put<AiSettings>("/ai-settings", request);
            runInAction(() => {
                this.aiSettings = response.data;
            });
        } catch (error) {
            console.error("Failed to update AI settings", error);
            this.rootStore.uiStore.setError("Failed to update AI settings");
            throw error;
        }
    };

    fetchSystemStats = async () => {
        this.isSystemStatsLoading = true;
        try {
            const response = await axios.get<SystemStats>("/system-stats");
            runInAction(() => {
                this.systemStats = response.data;
            });
        } catch (error) {
            console.error("Failed to fetch system stats", error);
        } finally {
            runInAction(() => {
                this.isSystemStatsLoading = false;
            });
        }
    };

    fetchP2PDaemonStatus = async () => {
        this.isP2PDaemonStatusLoading = true;
        try {
            const response = await axios.get<P2PDaemonStatus>("/p2p-daemon-status");
            runInAction(() => {
                this.p2pDaemonStatus = response.data;
            });
        } catch (error) {
            console.error("Failed to fetch P2P status", error);
            runInAction(() => {
                this.p2pDaemonStatus = null;
            });
        } finally {
            runInAction(() => {
                this.isP2PDaemonStatusLoading = false;
            });
        }
    };

    fetchP2PBackupStatus = async () => {
        this.isP2PBackupStatsLoading = true;
        try {
            const response = await axios.get<P2PBackupStatus>("/p2p/backup/status");
            runInAction(() => {
                this.p2pBackupStatus = response.data;
            });
        } catch (error) {
            console.error("Failed to fetch P2P backup status", error);
        } finally {
            runInAction(() => {
                this.isP2PBackupStatsLoading = false;
            });
        }
    };

    fetchDiscoveredPeers = async () => {
        this.isDiscoveredPeersLoading = true;
        try {
            const response = await axios.get<DiscoveredPeersResponse>("/p2p-discovered-peers");
            runInAction(() => {
                this.discoveredPeers = response.data.peers;
            });
        } catch (error) {
            console.error("Failed to fetch discovered peers", error);
        } finally {
            runInAction(() => {
                this.isDiscoveredPeersLoading = false;
            });
        }
    };

    verifyP2PBackup = async () => {
        try {
            runInAction(() => {
                this.verificationResult = null;
            });

            const response = await axios.get<VerificationResult>("/p2p/backup/verify");

            runInAction(() => {
                this.verificationResult = response.data;
            });

            if (response.data.failed_files > 0 || response.data.missing_files > 0) {
                this.rootStore.uiStore.setError(
                    `Verification found issues: ${response.data.failed_files} failed, ${response.data.missing_files} missing`
                );
            } else if (response.data.verified_files > 0) {
                this.rootStore.uiStore.setSuccess(
                    `All ${response.data.verified_files} files verified successfully!`
                );
            }
        } catch (error: any) {
            console.error("Failed to verify P2P backup", error);
            this.rootStore.uiStore.setError(
                error.response?.data?.error || "Failed to verify backup"
            );
            throw error;
        }
    };

    fetchAllStats = async () => {
        await Promise.all([
            this.fetchStats(),
            this.fetchPoolStats(),
            this.fetchGeoDbStats(),
            this.fetchAiSettings(),
            this.fetchSystemStats(),
            this.fetchP2PDaemonStatus(),
            this.fetchDiscoveredPeers(),
            this.fetchP2PBackupStatus(),
        ]);
    };
}
