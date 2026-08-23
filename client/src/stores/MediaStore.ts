import { makeAutoObservable, runInAction, reaction } from "mobx";
import type { RootStore } from "./RootStore";
import axios from "../api/axiosConfig";
import { logger } from "../utils/logger";

// Map paging: the backend caps /map/media responses, so the client pages
// through until it has every geotagged point (bounded by MAP_MAX_PAGES).
const MAP_PAGE_LIMIT = 10000;
const MAP_MAX_PAGES = 100;

export interface MapPoint {
    hash: string;
    lon: number;
    lat: number;
    created_at: string;
    place?: string | null;
    starred: boolean;
    device_id?: string | null;
    has_thumbnail: boolean;
}

export interface MediaItem {
    hash: string;
    name: string;
    created_at: string;
    place?: string;
    device_id?: string;
    thumbnailUrl?: string;
    starred?: boolean;
    similarity?: number; // Similarity score from search (0-1)
    distance_km?: number; // Distance from search location in kilometers
    media_type?: string; // "image" or "video"
    thumbnail_url?: string;
    file_size_bytes?: number;
    aesthetic_score?: number;
}

export type SearchType = 'semantic' | 'text' | 'hybrid';
export type MediaTypeFilter = 'all' | 'image' | 'video';

export interface LocationResult {
    name: string;
    latitude: number;
    longitude: number;
    admin_level: number;
    country_code: string | null;
    display_name: string;
}

export interface ImageMetadata {
    hash: string;
    name: string;
    description: string | null;
    place: string | null;
    created_at: string;
    exif: string | null;
    starred: boolean;
}

export interface MediaGroup {
    date: string; // YYYY-MM-DD format
    displayDate: string; // Human-readable format
    items: MediaItem[];
}

export interface ThumbnailsResponse {
    thumbnails: MediaItem[];
    total: number;
    page: number;
    limit: number;
}

export class MediaStore {
    rootStore: RootStore;
    
    // Monotonic counter used as a stale-response guard: when a fresh filter/search
    // starts we bump it, and any in-flight response whose captured seq is stale is
    // discarded so out-of-order responses can't overwrite newer results.
    private requestSeq = 0;
    private locationRequestSeq = 0;
    private metadataRequestSeq = 0;
    
    // Data Collections
    images: MediaItem[] = [];
    videos: MediaItem[] = [];
    allMedia: MediaItem[] = [];
    
    // Pagination & Meta
    currentPage: number = 1;
    totalImages: number = 0;
    hasMore: boolean = true;
    isLoadingMore: boolean = false;
    
    videoCurrentPage: number = 1;
    totalVideos: number = 0;
    videoHasMore: boolean = true;
    isLoadingMoreVideos: boolean = false;
    
    allMediaCurrentPage: number = 1;
    totalAllMedia: number = 0;
    allMediaHasMore: boolean = true;

    // Map view state
    mapActive: boolean = false;
    mapPoints: MapPoint[] = [];
    isMapLoading: boolean = false;
    mapTotal: number = 0;
    mapError: string = "";
    isLoadingMoreAllMedia: boolean = false;

    // "Orientation check" tab: images with no EXIF metadata (rely on AI orientation)
    noExifImages: MediaItem[] = [];
    noExifCurrentPage: number = 1;
    totalNoExif: number = 0;
    noExifHasMore: boolean = true;
    isLoadingNoExif: boolean = false;

    // View Preferences
    groupBy: 'day' | 'place' = 'day';
    videoGroupBy: 'day' | 'place' = 'day';
    allMediaGroupBy: 'day' | 'place' = 'day';
    sortBy: 'date' | 'size' | 'quality' = 'date';
    sortOrder: 'asc' | 'desc' = 'desc';

    // Centralized Filters
    filters = {
        selectedDeviceId: 'all',
        starredOnly: false,
        selectedLabelId: null as number | null,
        startDate: "",
        endDate: "",
        allMediaTypeFilter: 'all' as 'all' | 'image' | 'video',
        location: null as LocationResult | null,
        locationRadiusKm: 10,
    };

    // Search State
    searchQuery: string = "";
    searchMode: boolean = false;
    isSearching: boolean = false;
    isLoadingMoreSearch: boolean = false;
    searchOffset: number = 0;
    searchPageSize: number = 50;
    minSimilarity: number = 0.08;
    searchType: 'semantic' | 'text' | 'hybrid' = 'semantic';
    
    // Metadata State
    imageMetadata: ImageMetadata | null = null;
    lastLoadedMetadataHash: string | null = null;
    deviceIds: string[] = [];

    // Autocomplete State
    locationQuery: string = "";
    locationSuggestions: LocationResult[] = [];
    isLoadingLocationSuggestions: boolean = false;

    private similarityDebounceTimer: ReturnType<typeof setTimeout> | null = null;

    // Unified Lightbox State
    selectedMediaIndex: number | null = null;
    lightboxSource: 'all' | 'images' | 'videos' | 'custom' = 'all';
    customLightboxItems: MediaItem[] = [];
    fullMediaUrl: string | null = null;
    comparisonMediaUrl: string | null = null;
    compareMode: boolean = false;
    zoomScale: number = 1;
    zoomOffset: { x: number, y: number } = { x: 0, y: 0 };

    constructor(rootStore: RootStore) {
        makeAutoObservable(this);
        this.rootStore = rootStore;

        // MobX Reaction: Automatically refresh data when filters change
        // Search query is now triggered explicitly via Enter key
        reaction(
            () => ({ ...this.filters, searchType: this.searchType }),
            () => {
                if (this.searchMode && this.searchQuery.trim()) {
                    this.performSearch(this.searchQuery);
                } else {
                    this.applyFilters();
                }
            },
            { delay: 400 } // Debounce to prevent rapid API calls
        );
    }

    // --- Computed Values ---

    get activeLightboxItems(): MediaItem[] {
        if (this.lightboxSource === 'images') return this.images;
        if (this.lightboxSource === 'videos') return this.videos;
        if (this.lightboxSource === 'custom') return this.customLightboxItems;
        return this.allMedia;
    }

    get isFirstMedia(): boolean {
        return this.selectedMediaIndex === 0;
    }

    get isLastMedia(): boolean {
        return this.selectedMediaIndex !== null && this.selectedMediaIndex === this.activeLightboxItems.length - 1;
    }

    // --- Actions ---

    setAllMediaTypeFilter = (type: 'all' | 'image' | 'video') => {
        this.filters.allMediaTypeFilter = type;
    };

    setSelectedDeviceId = (deviceId: string) => {
        this.filters.selectedDeviceId = deviceId;
    };

    setStartDate = (date: string) => {
        this.filters.startDate = date;
    };

    setEndDate = (date: string) => {
        this.filters.endDate = date;
    };

    setSelectedLabelId = (id: number | null) => {
        this.filters.selectedLabelId = id;
    };

    toggleStarredFilter = () => {
        this.filters.starredOnly = !this.filters.starredOnly;
    };

    setSearchQuery = (query: string) => {
        this.searchQuery = query;
    };

    setSearchType = (type: 'semantic' | 'text' | 'hybrid') => {
        this.searchType = type;
    };

    setMinSimilarity = (value: number) => {
        this.minSimilarity = value;

        // Debounce the search to avoid excessive API calls while dragging the slider
        if (this.similarityDebounceTimer) {
            clearTimeout(this.similarityDebounceTimer);
        }

        // Re-search if currently in search mode (after 300ms delay)
        if (this.searchMode && this.searchQuery) {
            this.similarityDebounceTimer = setTimeout(() => {
                this.performSearch(this.searchQuery);
            }, 300);
        }
    };

    setGroupBy = (val: 'day' | 'place') => { this.groupBy = val; };
    setVideoGroupBy = (val: 'day' | 'place') => { this.videoGroupBy = val; };
    setSortBy = (val: 'date' | 'size' | 'quality') => { this.sortBy = val; this.applyFilters(); };
    setSortOrder = (val: 'asc' | 'desc') => { this.sortOrder = val; this.applyFilters(); };

    // --- Data Fetching ---

    applyFilters = () => {
        this.requestSeq += 1; // invalidate any in-flight responses
        runInAction(() => {
            this.searchMode = false;
            this.cleanupThumbnails();
            this.currentPage = 1;
            this.videoCurrentPage = 1;
            this.allMediaCurrentPage = 1;
            this.hasMore = true;
            this.videoHasMore = true;
            this.allMediaHasMore = true;
        });
        
        // Parallel fetch for all views
        this.fetchImages(1, 50, false);
        this.fetchVideos(1, 50, false);
        this.fetchAllMedia(1, 50, false);
        if (this.mapActive) this.fetchMapPoints();
    };

    fetchMapPoints = async () => {
        const seq = this.requestSeq;
        this.mapError = "";
        if (this.mapPoints.length === 0) this.isMapLoading = true;
        try {
            const params = new URLSearchParams({
                starred_only: this.filters.starredOnly.toString(),
                page: "1",
                limit: MAP_PAGE_LIMIT.toString(),
            });
            if (this.filters.startDate) params.append('start_date', this.filters.startDate);
            if (this.filters.endDate) params.append('end_date', this.filters.endDate);
            if (this.filters.selectedLabelId !== null) params.append('label_id', this.filters.selectedLabelId.toString());
            if (this.filters.selectedDeviceId !== 'all') params.append('device_id', this.filters.selectedDeviceId);

            // The server pages /map/media (capped at 10000/page); collect all
            // pages so supercluster can cluster the full geotagged library.
            // Use a keyset cursor (created_at + hash tiebreaker) so concurrent
            // inserts can't cause duplicate/missed points across pages.
            let collected: MapPoint[] = [];
            let total = 0;
            let page = 1;
            let afterCreatedAt: string | undefined;
            let afterHash: string | undefined;
            do {
                if (afterCreatedAt !== undefined && afterHash !== undefined) {
                    params.set("after_created_at", afterCreatedAt);
                    params.set("after_hash", afterHash);
                    params.delete("page");
                } else {
                    params.set("page", String(page));
                    params.delete("after_created_at");
                    params.delete("after_hash");
                }
                const response = await axios.get<{ points: MapPoint[]; total: number }>(`/map/media?${params}`);
                if (seq !== this.requestSeq) return; // stale response — discard
                total = response.data.total;
                const points = response.data.points;
                if (points.length === 0) break;
                const last = points[points.length - 1];
                afterCreatedAt = last.created_at;
                afterHash = last.hash;
                collected = [...collected, ...points];
                page += 1;
            } while (collected.length < total && page <= MAP_MAX_PAGES);

            runInAction(() => {
                this.mapPoints = collected;
                this.mapTotal = total;
                this.isMapLoading = false;
            });
        } catch (error) {
            if (seq === this.requestSeq) {
                logger.error("Failed to load map points", error);
                this.mapError = "Failed to load map";
                this.isMapLoading = false;
            }
        }
    };

    setMapActive = (active: boolean) => {
        this.mapActive = active;
        if (active && !this.searchMode) this.fetchMapPoints();
    };

    openMapPhoto = (hash: string) => {
        const point = this.mapPoints.find(p => p.hash === hash);
        if (!point) return;
        const item: MediaItem = {
            hash: point.hash,
            name: "",
            created_at: point.created_at,
            place: point.place ?? undefined,
            starred: point.starred,
            thumbnailUrl: `/api/thumbnail/${point.hash}`,
            media_type: "image",
        };
        this.customLightboxItems = [item];
        this.openMediaLightbox(0, "custom");
    };

    clearAllFilters = () => {
        runInAction(() => {
            this.searchQuery = "";
            this.filters.startDate = "";
            this.filters.endDate = "";
            this.filters.location = null;
            this.locationQuery = "";
            this.filters.starredOnly = false;
            this.filters.selectedLabelId = null;
            this.filters.allMediaTypeFilter = 'all';
            this.filters.selectedDeviceId = 'all';
            this.minSimilarity = 0.08;
            this.filters.locationRadiusKm = 10;
            this.sortBy = 'date';
            this.sortOrder = 'desc';
        });
    };

    clearSearch = () => {
        this.searchQuery = "";
    };

    performSearch = async (query: string, append: boolean = false) => {
        if (!query.trim()) return;

        if (append) {
            this.isLoadingMoreSearch = true;
        } else {
            this.requestSeq += 1; // invalidate any in-flight response
            this.isSearching = true;
            this.searchMode = true;
            this.searchOffset = 0;
        }
        const reqSeq = this.requestSeq;

        const offset = append ? this.searchOffset : 0;
        // Guard against an append past the server-reported total (stale hasMore).
        if (offset > this.totalAllMedia) {
            runInAction(() => { this.allMediaHasMore = false; this.isLoadingMoreSearch = false; });
            return;
        }

        try {
            const params = new URLSearchParams({
                query,
                limit: this.searchPageSize.toString(),
                offset: offset.toString(),
                min_similarity: this.minSimilarity.toString(),
                mode: this.searchType,
                media_type: this.filters.allMediaTypeFilter,
            });

            if (this.filters.selectedDeviceId !== 'all') params.append('device_id', this.filters.selectedDeviceId);
            if (this.filters.starredOnly) params.append('starred_only', 'true');
            if (this.filters.startDate) params.append('start_date', this.filters.startDate);
            if (this.filters.endDate) params.append('end_date', this.filters.endDate);
            if (this.filters.selectedLabelId !== null) params.append('label_id', this.filters.selectedLabelId.toString());
            if (this.filters.location) {
                params.append('location_lat', this.filters.location.latitude.toString());
                params.append('location_lon', this.filters.location.longitude.toString());
                params.append('location_radius_km', this.filters.locationRadiusKm.toString());
            }

            const response = await axios.get(`/search/images?${params}`);
            const itemsWithThumbnails = this.attachThumbnails(response.data.results.map((item: MediaItem) => ({
                ...item,
                thumbnailUrl: item.thumbnail_url ? this.getAuthenticatedUrl(item.thumbnail_url) : undefined
            })));
            // More results exist iff the server reports more than we have now
            // (offset + this page's rows). Do NOT rely on "did we get a full page" —
            // the +1 probe on the server makes an exhausted page shorter than limit.
            const hasMoreFromServer = response.data.total > offset + itemsWithThumbnails.length;
            if (reqSeq !== this.requestSeq) return; // stale response — discard

            runInAction(() => {
                if (append) {
                    this.allMedia = [...this.allMedia, ...itemsWithThumbnails];
                    this.images = [...this.images, ...itemsWithThumbnails];
                } else {
                    this.images = itemsWithThumbnails;
                    this.allMedia = itemsWithThumbnails;
                }
                this.totalImages = response.data.total;
                this.totalAllMedia = response.data.total;
                this.searchOffset = offset + itemsWithThumbnails.length;
                this.allMediaHasMore = hasMoreFromServer;
                this.hasMore = false;
            });
        } catch (error) {
            logger.error("Search failed", error);
            this.rootStore.uiStore.setError("Search failed");
        } finally {
            runInAction(() => {
                this.isSearching = false;
                this.isLoadingMoreSearch = false;
            });
        }
    };

    // Thumbnails are served same-origin from a stable URL and authenticated by the
    // httpOnly session cookie, so a plain <img src> works — no per-item blob fetch
    // waterfall (and no object-URL bookkeeping) is needed.
    private attachThumbnails = (items: MediaItem[]): MediaItem[] =>
        items.map(item => (
            item.thumbnailUrl
                ? item
                : { ...item, thumbnailUrl: this.getAuthenticatedUrl(`/api/thumbnail/${item.hash}`) }
        ));

    fetchImages = async (page: number = 1, limit: number = 50, append: boolean = false) => {
        const seq = this.requestSeq;
        if (!append) this.rootStore.uiStore.setLoading(true);
        else this.isLoadingMore = true;

        try {
            const params = new URLSearchParams({
                page: page.toString(),
                limit: limit.toString(),
                starred_only: this.filters.starredOnly.toString()
            });
            if (this.sortBy === 'size') params.append('sort_by', 'size');
            if (this.sortBy === 'quality') params.append('sort_by', 'quality');
            if (this.sortOrder === 'asc') params.append('sort_order', 'asc');

            if (this.filters.startDate) params.append('start_date', this.filters.startDate);
            if (this.filters.endDate) params.append('end_date', this.filters.endDate);
            if (this.filters.selectedLabelId !== null) params.append('label_id', this.filters.selectedLabelId.toString());
            if (this.filters.selectedDeviceId !== 'all') params.append('device_id', this.filters.selectedDeviceId);
            if (this.filters.location) {
                params.append('location_lat', this.filters.location.latitude.toString());
                params.append('location_lon', this.filters.location.longitude.toString());
                params.append('location_radius_km', this.filters.locationRadiusKm.toString());
            }

            const response = await axios.get<ThumbnailsResponse>(`/image_thumbnails?${params}`);
            const withUrls = this.attachThumbnails(response.data.thumbnails.map(t => ({
                ...t,
                thumbnailUrl: t.thumbnail_url ? this.getAuthenticatedUrl(t.thumbnail_url) : undefined
            })));
            if (seq !== this.requestSeq) return; // stale response — discard

            runInAction(() => {
                this.images = append ? [...this.images, ...withUrls] : withUrls;
                this.currentPage = response.data.page;
                this.totalImages = response.data.total;
                this.hasMore = this.images.length < response.data.total;
            });
        } catch  {
            this.rootStore.uiStore.setError("Failed to fetch images");
        } finally {
            runInAction(() => { this.isLoadingMore = false; this.rootStore.uiStore.setLoading(false); });
        }
    };

    fetchVideos = async (page: number = 1, limit: number = 50, append: boolean = false) => {
        const seq = this.requestSeq;
        if (append) this.isLoadingMoreVideos = true;

        try {
            const params = new URLSearchParams({
                page: page.toString(),
                limit: limit.toString(),
                starred_only: this.filters.starredOnly.toString()
            });
            if (this.sortBy === 'size') params.append('sort_by', 'size');
            if (this.sortBy === 'quality') params.append('sort_by', 'quality');
            if (this.sortOrder === 'asc') params.append('sort_order', 'asc');

            if (this.filters.startDate) params.append('start_date', this.filters.startDate);
            if (this.filters.endDate) params.append('end_date', this.filters.endDate);
            if (this.filters.selectedLabelId !== null) params.append('label_id', this.filters.selectedLabelId.toString());
            if (this.filters.selectedDeviceId !== 'all') params.append('device_id', this.filters.selectedDeviceId);
            if (this.filters.location) {
                params.append('location_lat', this.filters.location.latitude.toString());
                params.append('location_lon', this.filters.location.longitude.toString());
                params.append('location_radius_km', this.filters.locationRadiusKm.toString());
            }

            const response = await axios.get<ThumbnailsResponse>(`/video_thumbnails?${params}`);
            const withUrls = this.attachThumbnails(response.data.thumbnails.map(t => ({
                ...t,
                thumbnailUrl: t.thumbnail_url ? this.getAuthenticatedUrl(t.thumbnail_url) : undefined
            })));
            if (seq !== this.requestSeq) return; // stale response — discard

            runInAction(() => {
                this.videos = append ? [...this.videos, ...withUrls] : withUrls;
                this.videoCurrentPage = response.data.page;
                this.totalVideos = response.data.total;
                this.videoHasMore = this.videos.length < response.data.total;
            });
        } catch (error) {
            logger.error("Failed to fetch videos", error);
        } finally {
            runInAction(() => { this.isLoadingMoreVideos = false; });
        }
    };

    fetchAllMedia = async (page: number = 1, limit: number = 50, append: boolean = false) => {
        const seq = this.requestSeq;
        if (append) this.isLoadingMoreAllMedia = true;

        try {
            const params = new URLSearchParams({
                page: page.toString(),
                limit: limit.toString(),
                starred_only: this.filters.starredOnly.toString()
            });
            if (this.sortBy === 'size') params.append('sort_by', 'size');
            if (this.sortBy === 'quality') params.append('sort_by', 'quality');
            if (this.sortOrder === 'asc') params.append('sort_order', 'asc');

            if (this.filters.startDate) params.append('start_date', this.filters.startDate);
            if (this.filters.endDate) params.append('end_date', this.filters.endDate);
            if (this.filters.selectedLabelId !== null) params.append('label_id', this.filters.selectedLabelId.toString());
            if (this.filters.selectedDeviceId !== 'all') params.append('device_id', this.filters.selectedDeviceId);
            if (this.filters.location) {
                params.append('location_lat', this.filters.location.latitude.toString());
                params.append('location_lon', this.filters.location.longitude.toString());
                params.append('location_radius_km', this.filters.locationRadiusKm.toString());
            }

            let endpoint = '/media_thumbnails';
            if (this.filters.allMediaTypeFilter === 'image') endpoint = '/image_thumbnails';
            if (this.filters.allMediaTypeFilter === 'video') endpoint = '/video_thumbnails';

            const response = await axios.get<ThumbnailsResponse>(`${endpoint}?${params}`);
            const withUrls = this.attachThumbnails(response.data.thumbnails.map(t => ({
                ...t,
                thumbnailUrl: t.thumbnail_url ? this.getAuthenticatedUrl(t.thumbnail_url) : undefined
            })));
            if (seq !== this.requestSeq) return; // stale response — discard

            runInAction(() => {
                this.allMedia = append ? [...this.allMedia, ...withUrls] : withUrls;
                this.allMediaCurrentPage = response.data.page;
                this.totalAllMedia = response.data.total;
                this.allMediaHasMore = this.allMedia.length < response.data.total;
            });
        } catch (error) {
            logger.error("Failed to fetch all media", error);
        } finally {
            runInAction(() => { this.isLoadingMoreAllMedia = false; });
        }
    };

    loadMoreImages = () => { if (this.hasMore && !this.isLoadingMore) this.fetchImages(this.currentPage + 1, 50, true); };
    loadMoreVideos = () => { if (this.videoHasMore && !this.isLoadingMoreVideos) this.fetchVideos(this.videoCurrentPage + 1, 50, true); };
    loadMoreAllMedia = () => {
        if (!this.allMediaHasMore) return;
        if (this.searchMode) {
            if (!this.isSearching && !this.isLoadingMoreSearch) this.performSearch(this.searchQuery, true);
        } else {
            if (!this.isLoadingMoreAllMedia) this.fetchAllMedia(this.allMediaCurrentPage + 1, 50, true);
        }
    };

    // "Orientation check" tab — images with no EXIF metadata
    fetchNoExifImages = async (page: number = 1, limit: number = 50, append: boolean = false) => {
        const seq = this.requestSeq;
        if (append) this.isLoadingNoExif = true;
        else this.rootStore.uiStore.setLoading(true);

        try {
            const params = new URLSearchParams({
                page: page.toString(),
                limit: limit.toString(),
                no_exif: "true"
            });
            const response = await axios.get<ThumbnailsResponse>(`/image_thumbnails?${params}`);
            const withUrls = this.attachThumbnails(response.data.thumbnails.map(t => ({
                ...t,
                thumbnailUrl: t.thumbnail_url ? this.getAuthenticatedUrl(t.thumbnail_url) : undefined
            })));
            if (seq !== this.requestSeq) return;

            runInAction(() => {
                this.noExifImages = append ? [...this.noExifImages, ...withUrls] : withUrls;
                this.noExifCurrentPage = response.data.page;
                this.totalNoExif = response.data.total;
                this.noExifHasMore = this.noExifImages.length < response.data.total;
            });
        } catch {
            this.rootStore.uiStore.setError("Failed to fetch no-EXIF images");
        } finally {
            runInAction(() => {
                this.isLoadingNoExif = false;
                this.rootStore.uiStore.setLoading(false);
            });
        }
    };

    loadMoreNoExif = () => {
        if (!this.noExifHasMore || this.isLoadingNoExif) return;
        this.fetchNoExifImages(this.noExifCurrentPage + 1, 50, true);
    };

    // --- Lightbox Methods ---

    toggleCompareMode = async () => {
        this.compareMode = !this.compareMode;
        if (this.compareMode) {
            this.resetZoom();
            if (this.selectedMediaIndex !== null && this.selectedMediaIndex < this.activeLightboxItems.length - 1) {
                await this.loadComparisonMedia(this.selectedMediaIndex + 1);
            }
        } else {
            this.clearComparisonMedia();
        }
    };

    setZoomScale = (scale: number) => { this.zoomScale = Math.max(1, Math.min(scale, 10)); };
    setZoomOffset = (x: number, y: number) => { this.zoomOffset = { x, y }; };
    resetZoom = () => { this.zoomScale = 1; this.zoomOffset = { x: 0, y: 0 }; };

    openMediaLightbox = async (index: number, source: 'all' | 'images' | 'videos' | 'custom' = 'all') => {
        this.lightboxSource = source;
        this.selectedMediaIndex = index;
        this.resetZoom();
        await this.loadFullMedia(index);
    };

    closeMediaLightbox = () => {
        this.selectedMediaIndex = null;
        this.fullMediaUrl = null;
        this.comparisonMediaUrl = null;
        this.compareMode = false;
        this.resetZoom();
        this.imageMetadata = null;
        this.lastLoadedMetadataHash = null;
    };

    nextMedia = async () => {
        if (!this.isLastMedia && this.selectedMediaIndex !== null) {
            this.selectedMediaIndex++;
            this.resetZoom();
            await this.loadFullMedia(this.selectedMediaIndex);
            if (this.compareMode && this.selectedMediaIndex < this.activeLightboxItems.length - 1) {
                await this.loadComparisonMedia(this.selectedMediaIndex + 1);
            }
        }
    };

    previousMedia = async () => {
        if (!this.isFirstMedia && this.selectedMediaIndex !== null) {
            this.selectedMediaIndex--;
            this.resetZoom();
            await this.loadFullMedia(this.selectedMediaIndex);
            if (this.compareMode && this.selectedMediaIndex < this.activeLightboxItems.length - 1) {
                await this.loadComparisonMedia(this.selectedMediaIndex + 1);
            }
        }
    };

    loadFullMedia = async (index: number) => {
        const item = this.activeLightboxItems[index];
        if (!item) return;

        try {
            const url = this.getAuthenticatedUrl(
                item.media_type === 'video' ? `/api/video/${item.hash}` : `/api/image/${item.hash}`
            );
            runInAction(() => {
                this.fullMediaUrl = url;
            });
            if (item.media_type !== 'video') {
                await this.loadImageMetadata(item.hash);
            } else {
                runInAction(() => { this.clearImageMetadata(); });
            }
        } catch (error) {
            logger.error("Failed to load full media", error);
            this.rootStore.uiStore.setError("Failed to load media");
        }
    };

    loadComparisonMedia = async (index: number) => {
        const item = this.activeLightboxItems[index];
        if (!item) return;
        try {
            const url = this.getAuthenticatedUrl(
                item.media_type === 'video' ? `/api/video/${item.hash}` : `/api/image/${item.hash}`
            );
            runInAction(() => { this.comparisonMediaUrl = url; });
        } catch (error) { logger.error("Failed to load comparison", error); }
    };

    clearComparisonMedia = () => {
        this.comparisonMediaUrl = null;
    };

    // --- Metadata Actions ---

    loadImageMetadata = async (hash: string) => {
        const seq = ++this.metadataRequestSeq;
        try {
            const response = await axios.get<ImageMetadata>(`/image/${hash}/metadata`);
            if (seq !== this.metadataRequestSeq) return; // stale response — discard
            runInAction(() => { this.imageMetadata = response.data; this.lastLoadedMetadataHash = hash; });
        } catch (error) { logger.error("Metadata fetch failed", error); }
    };

    clearImageMetadata = () => { this.imageMetadata = null; this.lastLoadedMetadataHash = null; };

    toggleStarMedia = async (hash: string, deviceId?: string) => {
        // Identity is (hash, device_id): the same hash can exist on several
        // devices. Items without a device_id compare as "".
        const d = deviceId;
        const sameItem = (a: MediaItem) => a.hash === hash && (a.device_id ?? "") === (d ?? "");

        // Find the item in any array
        const item = this.images.find(sameItem) || this.videos.find(sameItem) || this.allMedia.find(sameItem);
        if (!item) return;

        const previousStarred = !!item.starred;
        const newStarred = !previousStarred;

        // Update all occurrences of this item across all arrays
        runInAction(() => {
            // Update in images array
            const imageItem = this.images.find(sameItem);
            if (imageItem) imageItem.starred = newStarred;

            // Update in videos array
            const videoItem = this.videos.find(sameItem);
            if (videoItem) videoItem.starred = newStarred;

            // Update in allMedia array
            const allMediaItem = this.allMedia.find(sameItem);
            if (allMediaItem) allMediaItem.starred = newStarred;

            // Update metadata if open in lightbox
            if (this.imageMetadata?.hash === hash) this.imageMetadata.starred = newStarred;
        });

        try {
            const endpoint = item.media_type === 'video' ? 'video' : 'image';
            const response = await axios.post(`/${endpoint}/${hash}/star`);

            // Update with server response
            runInAction(() => {
                const starred = response.data.starred;
                const imageItem = this.images.find(sameItem);
                if (imageItem) imageItem.starred = starred;

                const videoItem = this.videos.find(sameItem);
                if (videoItem) videoItem.starred = starred;

                const allMediaItem = this.allMedia.find(sameItem);
                if (allMediaItem) allMediaItem.starred = starred;

                if (this.imageMetadata?.hash === hash) this.imageMetadata.starred = starred;
            });
        } catch  {
            // Rollback on error
            runInAction(() => {
                const imageItem = this.images.find(sameItem);
                if (imageItem) imageItem.starred = previousStarred;

                const videoItem = this.videos.find(sameItem);
                if (videoItem) videoItem.starred = previousStarred;

                const allMediaItem = this.allMedia.find(sameItem);
                if (allMediaItem) allMediaItem.starred = previousStarred;

                if (this.imageMetadata?.hash === hash) this.imageMetadata.starred = previousStarred;
            });
            this.rootStore.uiStore.setError("Failed to update star status");
        }
    };

    deleteMedia = async (hash: string) => {
        const item = this.images.find(i => i.hash === hash) || 
                     this.videos.find(v => v.hash === hash) || 
                     this.allMedia.find(i => i.hash === hash) ||
                     this.customLightboxItems.find(i => i.hash === hash);
        if (!item) return;

        // Perform removal from all lists
        runInAction(() => {
            const filterOut = (list: MediaItem[]) => list.filter(i => i.hash !== hash);
            this.images = filterOut(this.images);
            this.videos = filterOut(this.videos);
            this.allMedia = filterOut(this.allMedia);
            this.customLightboxItems = filterOut(this.customLightboxItems);
            
            // Adjust lightbox index if necessary
            if (this.selectedMediaIndex !== null) {
                if (this.activeLightboxItems.length === 0) this.closeMediaLightbox();
                else {
                    if (this.selectedMediaIndex >= this.activeLightboxItems.length) this.selectedMediaIndex = this.activeLightboxItems.length - 1;
                    this.loadFullMedia(this.selectedMediaIndex);
                }
            }
        });

        try {
            const endpoint = item.media_type === 'video' ? 'video' : 'image';
            await axios.post(`/${endpoint}/${hash}/delete`);
        } catch  {
            this.rootStore.uiStore.setError("Deletion failed");
            this.applyFilters(); // Full refresh on error
        }
    };

    // --- Helper Methods ---

    // Media URLs are authenticated by the httpOnly session cookie (same-origin requests),
    // so no token ever needs to appear in the URL query string (which would leak into
    // nginx logs / history / Referer).
    getAuthenticatedUrl = (baseUrl: string) => baseUrl;

    cleanupThumbnails = () => {
        runInAction(() => { this.images = []; this.videos = []; this.allMedia = []; });
    };

    fetchDeviceIds = async () => {
        try {
            const response = await axios.get<{ device_ids: string[] }>('/device_ids');
            runInAction(() => {
                this.deviceIds = response.data.device_ids;
                if (this.rootStore.authStore.user?.role !== 'admin' && this.deviceIds.length === 1) {
                    this.filters.selectedDeviceId = this.deviceIds[0];
                }
            });
        } catch (error) { logger.error("Device ID fetch failed", error); }
    };

    // --- Getters for UI ---

    get filteredImages(): MediaItem[] {
        return this.filters.selectedDeviceId === 'all' ? this.images : this.images.filter(i => i.device_id === this.filters.selectedDeviceId);
    }

    get filteredVideos(): MediaItem[] {
        return this.filters.selectedDeviceId === 'all' ? this.videos : this.videos.filter(i => i.device_id === this.filters.selectedDeviceId);
    }

    get filteredAllMedia(): MediaItem[] {
        return this.filters.selectedDeviceId === 'all' ? this.allMedia : this.allMedia.filter(i => i.device_id === this.filters.selectedDeviceId);
    }

    get groupedImages(): MediaGroup[] { return this.groupMedia(this.filteredImages, this.groupBy); }
    get groupedVideos(): MediaGroup[] { return this.groupMedia(this.filteredVideos, this.videoGroupBy); }
    get groupedAllMedia(): MediaGroup[] { return this.groupMedia(this.filteredAllMedia, this.allMediaGroupBy); }

    private groupMedia(items: MediaItem[], mode: 'day' | 'place'): MediaGroup[] {
        const groups = new Map<string, MediaItem[]>();
        items.forEach(item => {
            const key = mode === 'day' ? new Date(item.created_at).toISOString().split('T')[0] : (item.place || 'Unknown Location');
            if (!groups.has(key)) groups.set(key, []);
            groups.get(key)!.push(item);
        });

        const sortBySize = this.sortBy === 'size';
        const sortByQuality = this.sortBy === 'quality';
        const asc = this.sortOrder === 'asc';
        const cmp = (a: number, b: number) => asc ? a - b : b - a;
        const grouped = Array.from(groups.entries()).map(([key, groupItems]) => ({
            date: key,
            displayDate: mode === 'day' ? this.formatDisplayDate(key) : key,
            items: sortBySize
                ? [...groupItems].sort((a, b) => cmp(a.file_size_bytes ?? 0, b.file_size_bytes ?? 0))
                : sortByQuality
                    ? [...groupItems].sort((a, b) => cmp(a.aesthetic_score ?? -1, b.aesthetic_score ?? -1))
                    : [...groupItems].sort((a, b) => cmp(new Date(a.created_at).getTime(), new Date(b.created_at).getTime()))
        }));

        if (sortBySize) {
            return [...grouped].sort((a, b) => {
                const maxA = Math.max(...a.items.map(i => i.file_size_bytes ?? 0));
                const maxB = Math.max(...b.items.map(i => i.file_size_bytes ?? 0));
                return cmp(maxA, maxB);
            });
        }
        if (sortByQuality) {
            return [...grouped].sort((a, b) => {
                const maxA = Math.max(...a.items.map(i => i.aesthetic_score ?? -1));
                const maxB = Math.max(...b.items.map(i => i.aesthetic_score ?? -1));
                return cmp(maxA, maxB);
            });
        }
        return [...grouped].sort((a, b) => {
            const byDate = mode === 'day' ? (asc ? a.date.localeCompare(b.date) : b.date.localeCompare(a.date)) : a.displayDate.localeCompare(b.displayDate);
            return byDate;
        });
    }

    private formatDisplayDate(dateKey: string): string {
        const date = new Date(dateKey);
        const today = new Date().toISOString().split('T')[0];
        if (dateKey === today) return 'Today';
        return date.toLocaleDateString('en-US', { weekday: 'long', year: 'numeric', month: 'long', day: 'numeric' });
    }

    // --- Autocomplete ---

    setLocationQuery = (query: string) => {
        this.locationQuery = query;
        this.locationRequestSeq += 1; // invalidate any in-flight suggestion responses
        if (query.length >= 3) this.fetchLocationSuggestions(query, this.locationRequestSeq);
        else this.locationSuggestions = [];
    };

    fetchLocationSuggestions = async (query: string, seq: number) => {
        this.isLoadingLocationSuggestions = true;
        try {
            const response = await axios.get('/search/places', { params: { query, limit: 20 } });
            if (seq !== this.locationRequestSeq) return; // stale response — discard
            runInAction(() => { this.locationSuggestions = response.data; });
        } catch (error) {
            logger.error("Location suggestions fetch failed", error);
            this.rootStore.uiStore.setError("Failed to fetch location suggestions");
            if (seq === this.locationRequestSeq) {
                runInAction(() => { this.locationSuggestions = []; });
            }
        }
        finally {
            if (seq === this.locationRequestSeq) {
                runInAction(() => { this.isLoadingLocationSuggestions = false; });
            }
        }
    };

    selectLocation = (location: LocationResult) => {
        this.filters.location = location;
        this.locationSuggestions = [];
    };

    setLocationRadiusKm = (radius: number) => { this.filters.locationRadiusKm = radius; };

    clearLocationFilter = () => {
        runInAction(() => {
            this.locationQuery = "";
            this.filters.location = null;
            this.locationSuggestions = [];
        });
    };

    fetchRandomImage = async (starredOnly: boolean = false, labelIds: number[] = []): Promise<MediaItem | null> => {
        try {
            const params = new URLSearchParams();
            if (starredOnly) params.append('starred_only', 'true');
            if (labelIds.length > 0) params.append('label_ids', labelIds.join(','));
            const response = await axios.get<{hash: string, name: string, created_at: string, place?: string}>(`/image/random?${params.toString()}`);
            const item = response.data;
            return {
                hash: item.hash,
                name: item.name,
                created_at: item.created_at,
                place: item.place,
                thumbnailUrl: this.getAuthenticatedUrl(`/api/image/${item.hash}`)
            };
        } catch (error) {
            logger.error("Failed to fetch random image for presentation", error);
            return null;
        }
    };
}
