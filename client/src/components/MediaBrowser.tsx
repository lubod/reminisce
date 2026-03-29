import { useEffect, useRef, useCallback, useState } from "react";
import { observer } from "mobx-react-lite";
import { useStore } from "../stores/RootStore";
import { MediaLightbox } from "./MediaLightbox";
import { Star, Search, X, ChevronDown, ChevronUp, SlidersHorizontal, Play, MapPin } from "lucide-react";

export const MediaBrowser = observer(() => {
    const { mediaStore, labelStore } = useStore();
    const observerTarget = useRef<HTMLDivElement>(null);
    const [filtersExpanded, setFiltersExpanded] = useState(false);
    const [searchInput, setSearchInput] = useState(mediaStore.searchQuery);

    // Sync local input with store query (e.g. when cleared)
    useEffect(() => {
        setSearchInput(mediaStore.searchQuery);
    }, [mediaStore.searchQuery]);

    useEffect(() => {
        mediaStore.applyFilters(); // Initial load
        mediaStore.fetchDeviceIds();
        labelStore.fetchLabels();

        return () => {
            mediaStore.cleanupThumbnails();
        };
    }, [mediaStore, labelStore]);

    const handleObserver = useCallback((entries: IntersectionObserverEntry[]) => {
        const [target] = entries;
        if (target.isIntersecting && mediaStore.allMediaHasMore && !mediaStore.isLoadingMoreAllMedia) {
            mediaStore.loadMoreAllMedia();
        }
    }, [mediaStore]);

    useEffect(() => {
        const element = observerTarget.current;
        if (!element) return;
        const observer = new IntersectionObserver(handleObserver, { threshold: 0, rootMargin: "20px" });
        observer.observe(element);
        return () => observer.unobserve(element);
    }, [handleObserver]);

    return (
        <div>
            <div className="mb-6">
                {/* Search Status */}
                {mediaStore.searchMode && !mediaStore.isSearching && (
                    <div className="text-xs text-blue-400 mb-3 px-2">
                        <span className="font-semibold">{mediaStore.totalAllMedia}</span> result{mediaStore.totalAllMedia !== 1 ? 's' : ''} for "{mediaStore.searchQuery}"
                    </div>
                )}

                {/* Collapsible Filters */}
                <div className="border border-gray-700 rounded-lg mb-3">
                    <button
                        onClick={() => setFiltersExpanded(!filtersExpanded)}
                        className="w-full flex items-center justify-between px-4 py-3 bg-gray-800 hover:bg-gray-700 rounded-t-lg transition-colors"
                    >
                        <div className="flex items-center gap-2">
                            <SlidersHorizontal size={18} className="text-gray-400" />
                            <span className="text-sm font-medium text-gray-200">Filters & Search</span>
                        </div>
                        {filtersExpanded ? <ChevronUp size={18} className="text-gray-400" /> : <ChevronDown size={18} className="text-gray-400" />}
                    </button>

                    {filtersExpanded && (
                        <div className="p-4 bg-gray-800 space-y-4 md:space-y-0 md:grid md:grid-cols-5 md:gap-6 animate-in fade-in slide-in-from-top-2 duration-200">
                            <div className="space-y-3">
                                <div>
                                    <label className="text-xs font-medium text-gray-300 block mb-1">Search:</label>
                                    <form 
                                        onSubmit={(e) => {
                                            e.preventDefault();
                                            if (searchInput.trim()) {
                                                mediaStore.setSearchQuery(searchInput);
                                                mediaStore.performSearch(searchInput);
                                            } else {
                                                mediaStore.clearSearch();
                                                mediaStore.applyFilters();
                                            }
                                        }}
                                        className="relative"
                                    >
                                        <input
                                            type="text"
                                            placeholder="Search collection..."
                                            value={searchInput}
                                            onChange={(e) => setSearchInput(e.target.value)}
                                            className="w-full px-4 py-2 pl-10 pr-10 border border-gray-600 bg-gray-700 text-gray-100 rounded-md focus:ring-2 focus:ring-blue-500 text-sm"
                                        />
                                        <Search className="absolute left-3 top-1/2 -translate-y-1/2 text-gray-400" size={18} />
                                        {searchInput && (
                                            <button 
                                                type="button"
                                                onClick={() => {
                                                    setSearchInput("");
                                                    mediaStore.clearSearch();
                                                    mediaStore.applyFilters();
                                                }} 
                                                className="absolute right-3 top-1/2 -translate-y-1/2 text-gray-400 hover:text-white"
                                            >
                                                <X size={18} />
                                            </button>
                                        )}
                                    </form>
                                </div>
                                <div>
                                    <label className="text-xs font-medium text-gray-300 block mb-1">Mode:</label>
                                    <select
                                        value={mediaStore.searchType}
                                        onChange={(e) => mediaStore.setSearchType(e.target.value as any)}
                                        className="w-full px-3 py-2 bg-gray-700 border-gray-600 text-gray-100 rounded-md text-sm"
                                    >
                                        <option value="semantic">🤖 AI Semantic</option>
                                        <option value="text">📝 Text Keywords</option>
                                        <option value="hybrid">⚡ Hybrid</option>
                                    </select>
                                </div>
                                {mediaStore.searchType !== 'text' && (
                                    <div>
                                        <label className="text-xs font-medium text-gray-300 block mb-1">
                                            Min Similarity: {(mediaStore.minSimilarity * 100).toFixed(0)}
                                        </label>
                                        <input
                                            type="range"
                                            min="0"
                                            max="20"
                                            value={mediaStore.minSimilarity * 100}
                                            onChange={(e) => mediaStore.setMinSimilarity(parseInt(e.target.value) / 100)}
                                            className="w-full h-2 bg-gray-700 rounded-lg appearance-none cursor-pointer accent-blue-500"
                                        />
                                        <div className="flex justify-between text-[9px] text-gray-500 mt-0.5">
                                            <span>0</span>
                                            <span>10</span>
                                            <span>20</span>
                                        </div>
                                    </div>
                                )}
                            </div>

                            {/* Date Range */}
                            <div className="space-y-2">
                                <label className="text-xs font-medium text-gray-300 block">Date Range:</label>
                                <input
                                    type="date"
                                    value={mediaStore.filters.startDate}
                                    onChange={(e) => mediaStore.setStartDate(e.target.value)}
                                    className="w-full px-3 py-2 bg-gray-700 border-gray-600 text-gray-100 rounded-md text-sm"
                                />
                                <input
                                    type="date"
                                    value={mediaStore.filters.endDate}
                                    onChange={(e) => mediaStore.setEndDate(e.target.value)}
                                    className="w-full px-3 py-2 bg-gray-700 border-gray-600 text-gray-100 rounded-md text-sm"
                                />
                            </div>

                            {/* Location */}
                            <div className="space-y-2">
                                <label className="text-xs font-medium text-gray-300 block">Location:</label>
                                <div className="relative">
                                    <input
                                        type="text"
                                        placeholder="Find place..."
                                        value={mediaStore.locationQuery}
                                        onChange={(e) => mediaStore.setLocationQuery(e.target.value)}
                                        className="w-full px-3 py-2 bg-gray-700 border-gray-600 text-gray-100 rounded-md text-sm"
                                    />
                                    {mediaStore.locationSuggestions.length > 0 && (
                                        <div className="absolute z-50 w-full mt-1 bg-gray-800 border border-gray-600 rounded-md shadow-2xl max-h-48 overflow-auto">
                                            {mediaStore.locationSuggestions.map((loc, idx) => (
                                                <button key={idx} onClick={() => mediaStore.selectLocation(loc)} className="w-full px-3 py-2 text-left text-sm hover:bg-gray-700 border-b border-gray-700 text-gray-200">
                                                    {loc.name} <span className="text-[10px] text-gray-500 block">{loc.display_name}</span>
                                                </button>
                                            ))}
                                        </div>
                                    )}
                                </div>
                                {mediaStore.filters.location && (
                                    <div className="space-y-2">
                                        <div className="flex items-center justify-between bg-blue-900/20 border border-blue-500/30 p-2 rounded text-xs text-blue-300">
                                            <span className="truncate font-semibold">{mediaStore.filters.location.name}</span>
                                            <button onClick={() => mediaStore.clearLocationFilter()} className="p-1 hover:bg-blue-800/30 rounded transition-colors"><X size={14}/></button>
                                        </div>
                                        
                                        <div>
                                            <div className="flex justify-between items-center mb-1">
                                                <label className="text-[10px] uppercase font-bold text-gray-500 tracking-wider">Radius:</label>
                                                <span className="text-[10px] font-mono text-blue-400 bg-blue-900/20 px-1.5 py-0.5 rounded border border-blue-500/20">
                                                    {mediaStore.filters.locationRadiusKm} km
                                                </span>
                                            </div>
                                            <input
                                                type="range"
                                                min="1"
                                                max="500"
                                                step="1"
                                                value={mediaStore.filters.locationRadiusKm}
                                                onChange={(e) => mediaStore.setLocationRadiusKm(parseInt(e.target.value))}
                                                className="w-full h-1.5 bg-gray-700 rounded-lg appearance-none cursor-pointer accent-blue-500"
                                            />
                                            <div className="flex justify-between text-[8px] text-gray-600 mt-1 font-medium">
                                                <span>1km</span>
                                                <span>100km</span>
                                                <span>500km</span>
                                            </div>
                                        </div>
                                    </div>
                                )}
                            </div>

                            {/* Labels & Favorites */}
                            <div className="space-y-3">
                                <label className="text-xs font-medium text-gray-300 block">Organization:</label>
                                <select
                                    value={mediaStore.filters.selectedLabelId || ""}
                                    onChange={(e) => mediaStore.setSelectedLabelId(e.target.value ? parseInt(e.target.value) : null)}
                                    className="w-full px-3 py-2 bg-gray-700 border-gray-600 text-gray-100 rounded-md text-sm"
                                >
                                    <option value="">All Labels</option>
                                    {labelStore.labels.map(l => <option key={l.id} value={l.id}>{l.name}</option>)}
                                </select>
                                <button
                                    onClick={() => mediaStore.toggleStarredFilter()}
                                    className={`w-full py-2 rounded-md text-sm font-medium transition-all flex items-center justify-center gap-2 ${
                                        mediaStore.filters.starredOnly ? 'bg-yellow-600 text-white' : 'bg-gray-700 text-gray-300 border border-gray-600'
                                    }`}
                                >
                                    <Star size={16} className={mediaStore.filters.starredOnly ? 'fill-white' : ''} />
                                    Starred
                                </button>
                            </div>

                            {/* Device & Type */}
                            <div className="space-y-3">
                                <label className="text-xs font-medium text-gray-300 block">System:</label>
                                <select
                                    value={mediaStore.filters.allMediaTypeFilter}
                                    onChange={(e) => mediaStore.setAllMediaTypeFilter(e.target.value as any)}
                                    className="w-full px-3 py-2 bg-gray-700 border-gray-600 text-gray-100 rounded-md text-sm"
                                >
                                    <option value="all">All Types</option>
                                    <option value="image">Images Only</option>
                                    <option value="video">Videos Only</option>
                                </select>
                                <select
                                    value={mediaStore.filters.selectedDeviceId}
                                    onChange={(e) => mediaStore.setSelectedDeviceId(e.target.value)}
                                    className="w-full px-3 py-2 bg-gray-700 border-gray-600 text-gray-100 rounded-md text-sm"
                                >
                                    <option value="all">All Devices</option>
                                    {mediaStore.deviceIds.map(id => <option key={id} value={id}>{id}</option>)}
                                </select>
                            </div>
                        </div>
                    )}
                </div>

                <div className="flex justify-between items-center px-2">
                    <button onClick={() => mediaStore.clearAllFilters()} className="text-xs text-gray-500 hover:text-white underline">Reset Filters</button>
                    <div className="flex items-center gap-3">
                        <div className="flex items-center gap-1">
                            <span className="text-[10px] text-gray-500 uppercase tracking-widest font-bold">Sort:</span>
                            <button
                                onClick={() => mediaStore.setSortBy('date')}
                                className={`text-[10px] px-2 py-0.5 rounded transition-colors ${mediaStore.sortBy === 'date' ? 'bg-gray-600 text-white' : 'text-gray-500 hover:text-white'}`}
                            >Date</button>
                            <button
                                onClick={() => mediaStore.setSortBy('size')}
                                className={`text-[10px] px-2 py-0.5 rounded transition-colors ${mediaStore.sortBy === 'size' ? 'bg-gray-600 text-white' : 'text-gray-500 hover:text-white'}`}
                            >Size</button>
                            <button
                                onClick={() => mediaStore.setSortBy('quality')}
                                className={`text-[10px] px-2 py-0.5 rounded transition-colors ${mediaStore.sortBy === 'quality' ? 'bg-gray-600 text-white' : 'text-gray-500 hover:text-white'}`}
                            >Quality</button>
                            <span className="text-gray-600 mx-0.5">|</span>
                            <button
                                onClick={() => mediaStore.setSortOrder('desc')}
                                className={`text-[10px] px-2 py-0.5 rounded transition-colors ${mediaStore.sortOrder === 'desc' ? 'bg-gray-600 text-white' : 'text-gray-500 hover:text-white'}`}
                            >↓</button>
                            <button
                                onClick={() => mediaStore.setSortOrder('asc')}
                                className={`text-[10px] px-2 py-0.5 rounded transition-colors ${mediaStore.sortOrder === 'asc' ? 'bg-gray-600 text-white' : 'text-gray-500 hover:text-white'}`}
                            >↑</button>
                        </div>
                        <div className="text-[10px] text-gray-500 uppercase tracking-widest font-bold">
                            Showing {mediaStore.allMedia.length} / {mediaStore.totalAllMedia} items
                        </div>
                    </div>
                </div>
            </div>

            {mediaStore.groupedAllMedia.map((group) => (
                <div key={group.date} className="mb-8 animate-in fade-in slide-in-from-bottom-2 duration-500">
                    <h2 className="text-lg font-bold text-gray-200 border-b border-gray-800 pb-2 mb-4 flex items-center justify-between">
                        {group.displayDate}
                        <span className="text-xs font-normal text-gray-500">{group.items.length} items</span>
                    </h2>
                    <div className="grid grid-cols-3 md:grid-cols-4 lg:grid-cols-6 gap-2">
                        {group.items.map((item) => {
                            const actualIndex = mediaStore.allMedia.findIndex(m => m.hash === item.hash);
                            return (
                                <div
                                    key={item.hash}
                                    className="group relative aspect-square bg-gray-800 rounded-xl overflow-hidden cursor-pointer hover:ring-2 ring-blue-500 transition-all shadow-lg"
                                    onClick={() => mediaStore.openMediaLightbox(actualIndex, 'all')}
                                >
                                    {item.thumbnailUrl ? (
                                        <img src={item.thumbnailUrl} alt={item.name} className="object-cover w-full h-full group-hover:scale-110 transition-transform duration-500" loading="lazy" />
                                    ) : (
                                        <div className="flex items-center justify-center h-full text-gray-600 italic text-xs">Loading...</div>
                                    )}
                                    {item.media_type === 'video' && (
                                        <div className="absolute inset-0 flex items-center justify-center bg-black/20">
                                            <Play size={32} className="text-white fill-white/50" />
                                        </div>
                                    )}
                                    {/* Similarity Score Badge */}
                                    {item.similarity !== undefined && (
                                        <div className="absolute top-2 left-2 px-2 py-0.5 rounded-md bg-blue-500/90 backdrop-blur-sm">
                                            <span className="text-[10px] font-bold text-white">
                                                {(item.similarity * 100).toFixed(0)}
                                            </span>
                                        </div>
                                    )}
                                    {/* Star Button */}
                                    <button
                                        onClick={(e) => {
                                            e.stopPropagation();
                                            mediaStore.toggleStarMedia(item.hash);
                                        }}
                                        className={`absolute top-2 right-2 p-1.5 rounded-full transition-all ${
                                            item.starred
                                                ? 'bg-yellow-400/20 opacity-100'
                                                : 'bg-black/50 opacity-0 group-hover:opacity-100'
                                        } hover:bg-black/70 hover:scale-110`}
                                        title={item.starred ? "Unstar" : "Star"}
                                    >
                                        <Star
                                            size={14}
                                            className={item.starred ? "text-yellow-400 fill-yellow-400" : "text-white"}
                                        />
                                    </button>
                                    {item.place && (
                                        <div className="absolute bottom-0 left-0 right-0 p-1.5 bg-gradient-to-t from-black/90 via-black/70 to-transparent">
                                            <div className="flex items-center gap-1 text-white">
                                                <MapPin size={10} className="flex-shrink-0" />
                                                <p className="text-[9px] font-medium truncate leading-tight">{item.place}</p>
                                            </div>
                                        </div>
                                    )}
                                    <div className="absolute bottom-0 left-0 right-0 p-2 bg-gradient-to-t from-black/80 to-transparent opacity-0 group-hover:opacity-100 transition-opacity">
                                        <p className="text-[10px] text-white truncate">{item.place || item.name}</p>
                                    </div>
                                </div>
                            );
                        })}
                    </div>
                </div>
            ))}

            <div ref={observerTarget} className="h-20 flex items-center justify-center">
                {mediaStore.isLoadingMoreAllMedia && <div className="w-6 h-6 border-2 border-blue-500 border-t-transparent rounded-full animate-spin" />}
            </div>

            <MediaLightbox />
        </div>
    );
});