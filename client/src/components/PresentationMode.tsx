import { useEffect, useState, useCallback, useRef } from "react";
import { observer } from "mobx-react-lite";
import { useStore } from "../stores/RootStore";
import type { MediaItem } from "../stores/MediaStore";
import { Maximize2, Minimize2, Star, Tag, Settings, X, Play, Pause, Info } from "lucide-react";

export const PresentationMode = observer(() => {
    const { mediaStore, uiStore, labelStore } = useStore();
    const [currentImage, setCurrentImage] = useState<MediaItem | null>(null);
    const [nextImage, setNextImage] = useState<MediaItem | null>(null);
    const [timeLeft, setTimeLeft] = useState(15);
    const [opacity, setOpacity] = useState(0);
    const [error, setError] = useState(false);
    const [zoomDirection, setZoomDirection] = useState<'in' | 'out'>('in');
    const [orientation, setOrientation] = useState<'landscape' | 'portrait'>('landscape');
    const [currentTime, setCurrentTime] = useState(new Date());
    
    // Presentation Settings
    const [starredOnly, setStarredOnly] = useState(false);
    const [selectedLabelId, setSelectedLabelId] = useState<number | null>(null);
    const [isPaused, setIsPaused] = useState(false);
    const [showSettings, setShowSettings] = useState(false);
    const [showInfo, setShowInfo] = useState(true);
    
    const settingsRef = useRef<HTMLDivElement>(null);

    // Update clock every second
    useEffect(() => {
        const clockTimer = setInterval(() => {
            setCurrentTime(new Date());
        }, 1000);
        return () => clearInterval(clockTimer);
    }, []);

    // Fetch labels on mount
    useEffect(() => {
        labelStore.fetchLabels();
    }, [labelStore]);

    // Fetch a new random image
    const fetchNext = useCallback(async () => {
        const image = await mediaStore.fetchRandomImage(starredOnly, selectedLabelId);
        if (image) {
            setNextImage(image);
            setError(false);
        } else {
            // If we can't find a next image, but have a current one, just stay on current
            if (!currentImage) setError(true);
        }
    }, [mediaStore, starredOnly, selectedLabelId, currentImage]);

    // Initial load and reset on settings change
    useEffect(() => {
        const init = async () => {
            setCurrentImage(null);
            setNextImage(null);
            setError(false);
            setOpacity(0);
            
            const first = await mediaStore.fetchRandomImage(starredOnly, selectedLabelId);
            if (first) {
                setCurrentImage(first);
                setZoomDirection(Math.random() > 0.5 ? 'in' : 'out');
                setOpacity(1);
                setTimeLeft(15);
                
                // Pre-fetch the next one
                const next = await mediaStore.fetchRandomImage(starredOnly, selectedLabelId);
                if (next) setNextImage(next);
            } else {
                setError(true);
            }
        };
        init();

        return () => {
            // Cleanup blob URLs on unmount
            if (currentImage?.thumbnailUrl) URL.revokeObjectURL(currentImage.thumbnailUrl);
            if (nextImage?.thumbnailUrl) URL.revokeObjectURL(nextImage.thumbnailUrl);
            // Ensure we exit fullscreen state on unmount
            uiStore.setIsFullscreen(false);
        };
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [starredOnly, selectedLabelId]);

    // Timer logic
    useEffect(() => {
        if (error || isPaused) return;

        const timer = setInterval(() => {
            setTimeLeft((prev) => {
                if (prev <= 1) {
                    // Time's up, switch images
                    if (nextImage) {
                        setOpacity(0);
                        setTimeout(() => {
                            // Revoke old URL to prevent memory leaks
                            if (currentImage?.thumbnailUrl) {
                                URL.revokeObjectURL(currentImage.thumbnailUrl);
                            }
                            setCurrentImage(nextImage);
                            setZoomDirection(Math.random() > 0.5 ? 'in' : 'out');
                            setNextImage(null);
                            setOpacity(1);
                            fetchNext();
                        }, 1000); // Wait for fade out
                        return 15; // Reset timer
                    } else {
                        // If no next image yet, try fetching again
                        fetchNext();
                        return 2; // Check again in 2s
                    }
                }
                return prev - 1;
            });
        }, 1000);

        return () => clearInterval(timer);
    }, [currentImage, nextImage, fetchNext, error, isPaused]);

    const toggleFullscreen = () => {
        if (!document.fullscreenElement) {
            document.documentElement.requestFullscreen().then(() => {
                uiStore.setIsFullscreen(true);
            }).catch(err => {
                console.error(`Error attempting to enable full-screen mode: ${err.message}`);
            });
        } else {
            if (document.exitFullscreen) {
                document.exitFullscreen().then(() => {
                    uiStore.setIsFullscreen(false);
                });
            }
        }
    };

    // Close settings on click outside
    useEffect(() => {
        const handleClickOutside = (event: MouseEvent) => {
            if (settingsRef.current && !settingsRef.current.contains(event.target as Node)) {
                setShowSettings(false);
            }
        };
        if (showSettings) {
            document.addEventListener("mousedown", handleClickOutside);
        }
        return () => document.removeEventListener("mousedown", handleClickOutside);
    }, [showSettings]);

    // Listen for fullscreen change (e.g. Esc key)
    useEffect(() => {
        const handleFullscreenChange = () => {
            uiStore.setIsFullscreen(!!document.fullscreenElement);
        };
        document.addEventListener('fullscreenchange', handleFullscreenChange);
        return () => document.removeEventListener('fullscreenchange', handleFullscreenChange);
    }, [uiStore]);

    if (error) {
        return (
            <div className="flex flex-col items-center justify-center h-[80vh] text-gray-400 text-center bg-gray-900 rounded-lg border border-gray-800">
                <div className="max-w-md p-8">
                    <p className="text-xl font-bold text-gray-200 mb-4">No images found for these settings.</p>
                    <p className="text-sm mb-8">Try adjusting your filters or upload some new photos.</p>
                    
                    <div className="flex flex-col gap-4">
                        <button 
                            onClick={() => { setStarredOnly(false); setSelectedLabelId(null); }}
                            className="px-6 py-2 bg-blue-600 hover:bg-blue-700 text-white rounded-md transition-colors"
                        >
                            Clear All Filters
                        </button>
                    </div>
                </div>
            </div>
        );
    }

    if (!currentImage) {
        return (
            <div className="flex items-center justify-center h-[80vh] text-gray-400 bg-gray-900 rounded-lg">
                <div className="text-center">
                    <p className="text-xl mb-2">Loading Presentation...</p>
                    <div className="w-8 h-8 border-4 border-blue-500 border-t-transparent rounded-full animate-spin mx-auto"></div>
                </div>
            </div>
        );
    }

    return (
        <div className={`relative w-full overflow-hidden ${uiStore.isFullscreen ? 'h-screen fixed top-0 left-0 z-50 bg-black' : 'h-[calc(100vh-100px)] bg-gray-900 rounded-lg shadow-2xl border border-gray-800'}`}>
            
            {/* Image Display */}
            <div className="absolute inset-0 flex items-center justify-center overflow-hidden bg-black">
                {currentImage.thumbnailUrl && (
                    <img 
                        key={currentImage.hash}
                        src={currentImage.thumbnailUrl} 
                        alt={currentImage.name}
                        onLoad={(e) => {
                            const img = e.currentTarget;
                            setOrientation(img.naturalWidth > img.naturalHeight ? 'landscape' : 'portrait');
                        }}
                        style={{ opacity }}
                        className={`w-full h-full ${
                            orientation === 'landscape' ? 'object-cover' : 'object-contain'
                        } transition-opacity duration-1000 will-change-transform ${
                            zoomDirection === 'in' ? 'animate-slow-zoom-in' : 'animate-slow-zoom-out'
                        }`}
                    />
                )}
            </div>

            {/* Info Overlay */}
            <div className={`absolute bottom-0 left-0 right-0 bg-gradient-to-t from-black/90 via-black/40 to-transparent p-8 text-white transition-all duration-700 pointer-events-none ${opacity === 1 && showInfo ? 'opacity-100 translate-y-0' : 'opacity-0 translate-y-10'}`}>
                <div className="flex justify-between items-end max-w-6xl mx-auto">
                    <div>
                        <div className="flex items-center gap-2 mb-2">
                            {currentImage.starred && <Star size={18} className="text-yellow-400 fill-yellow-400" />}
                            <h2 className="text-3xl font-bold">{currentImage.place || "Unknown Location"}</h2>
                        </div>
                        <p className="text-gray-300 text-lg">
                            {new Date(currentImage.created_at).toLocaleDateString(undefined, {
                                weekday: 'long',
                                year: 'numeric',
                                month: 'long',
                                day: 'numeric'
                            })}
                        </p>
                    </div>
                    <div className="text-right">
                        <div className="text-xs text-gray-400 uppercase tracking-widest mb-1">Next slide in</div>
                        <div className="text-4xl font-mono font-bold text-blue-400">{timeLeft}s</div>
                    </div>
                </div>
            </div>

            {/* Top Left: Time and Status */}
            <div className={`absolute top-0 left-0 p-8 bg-gradient-to-b from-black/70 to-transparent w-full flex justify-between items-start pointer-events-none transition-all duration-700 ${showInfo ? 'opacity-100 translate-y-0' : 'opacity-0 -translate-y-10'}`}>
                <div className="text-white drop-shadow-2xl">
                    <div className="text-6xl font-bold font-mono tracking-tighter">
                        {currentTime.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit', hour12: false })}
                    </div>
                    <div className="text-lg font-medium opacity-90 mt-1">
                        {currentTime.toLocaleDateString(undefined, { weekday: 'short', day: 'numeric', month: 'short' })}
                    </div>
                </div>
            </div>

            {/* Controls Bar */}
            <div className="absolute top-6 right-6 flex gap-3 pointer-events-auto">
                <button 
                    onClick={() => setIsPaused(!isPaused)}
                    className="p-3 bg-black/40 hover:bg-black/60 rounded-full text-white backdrop-blur-md border border-white/10 transition-all active:scale-95"
                    title={isPaused ? "Resume" : "Pause"}
                >
                    {isPaused ? <Play size={24} fill="white" /> : <Pause size={24} fill="white" />}
                </button>
                <button 
                    onClick={() => setShowInfo(!showInfo)}
                    className={`p-3 rounded-full text-white backdrop-blur-md border transition-all active:scale-95 ${showInfo ? 'bg-indigo-600 border-indigo-400' : 'bg-black/40 hover:bg-black/60 border-white/10'}`}
                    title={showInfo ? "Hide Info" : "Show Info"}
                >
                    <Info size={24} />
                </button>
                <button 
                    onClick={() => setShowSettings(!showSettings)}
                    className={`p-3 rounded-full text-white backdrop-blur-md border transition-all active:scale-95 ${showSettings ? 'bg-blue-600 border-blue-400' : 'bg-black/40 hover:bg-black/60 border-white/10'}`}
                    title="Presentation Settings"
                >
                    <Settings size={24} />
                </button>
                <button 
                    onClick={toggleFullscreen}
                    className="p-3 bg-black/40 hover:bg-black/60 rounded-full text-white backdrop-blur-md border border-white/10 transition-all active:scale-95"
                    title={uiStore.isFullscreen ? "Exit Fullscreen" : "Enter Fullscreen"}
                >
                    {uiStore.isFullscreen ? <Minimize2 size={24} /> : <Maximize2 size={24} />}
                </button>
            </div>

            {/* Settings Dropdown */}
            {showSettings && (
                <div 
                    ref={settingsRef}
                    className="absolute top-20 right-6 w-72 bg-gray-900/95 backdrop-blur-xl border border-gray-700 rounded-xl shadow-2xl p-6 z-[60] animate-in fade-in zoom-in-95 duration-200"
                >
                    <div className="flex justify-between items-center mb-6">
                        <h3 className="font-bold text-gray-100 flex items-center gap-2">
                            <Settings size={18} className="text-blue-400" />
                            Presentation Settings
                        </h3>
                        <button onClick={() => setShowSettings(false)} className="text-gray-500 hover:text-white">
                            <X size={20} />
                        </button>
                    </div>

                    <div className="space-y-6">
                        {/* Info Toggle */}
                        <div>
                            <label className="flex items-center justify-between cursor-pointer group">
                                <span className="text-sm font-medium text-gray-300 group-hover:text-white transition-colors">Show Information</span>
                                <div 
                                    className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors ${showInfo ? 'bg-indigo-600' : 'bg-gray-700'}`}
                                    onClick={() => setShowInfo(!showInfo)}
                                >
                                    <span className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${showInfo ? 'translate-x-6' : 'translate-x-1'}`} />
                                </div>
                            </label>
                        </div>

                        {/* Starred Filter */}
                        <div>
                            <label className="flex items-center justify-between cursor-pointer group">
                                <span className="text-sm font-medium text-gray-300 group-hover:text-white transition-colors">Only Starred Images</span>
                                <div 
                                    className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors ${starredOnly ? 'bg-blue-600' : 'bg-gray-700'}`}
                                    onClick={() => setStarredOnly(!starredOnly)}
                                >
                                    <span className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${starredOnly ? 'translate-x-6' : 'translate-x-1'}`} />
                                </div>
                            </label>
                        </div>

                        {/* Label Filter */}
                        <div className="space-y-2">
                            <label className="text-sm font-medium text-gray-300 block">Filter by Label</label>
                            <div className="relative">
                                <select 
                                    value={selectedLabelId || ""} 
                                    onChange={(e) => setSelectedLabelId(e.target.value ? parseInt(e.target.value) : null)}
                                    className="w-full bg-gray-800 border border-gray-700 rounded-lg px-3 py-2 text-sm text-gray-200 focus:ring-2 focus:ring-blue-500 focus:border-transparent outline-none transition-all appearance-none"
                                >
                                    <option value="">All Labels</option>
                                    {labelStore.labels.map(label => (
                                        <option key={label.id} value={label.id}>{label.name}</option>
                                    ))}
                                </select>
                                <div className="absolute right-3 top-1/2 -translate-y-1/2 pointer-events-none text-gray-500">
                                    <Tag size={14} />
                                </div>
                            </div>
                        </div>

                        <div className="pt-4 border-t border-gray-800">
                            <div className="flex items-center gap-2 text-[10px] text-gray-500 uppercase tracking-widest font-bold">
                                Current View Status
                            </div>
                            <div className="mt-2 flex flex-wrap gap-2">
                                <span className={`px-2 py-1 rounded text-[10px] font-bold border ${starredOnly ? 'bg-yellow-900/20 border-yellow-700/50 text-yellow-500' : 'bg-gray-800 border-gray-700 text-gray-400'}`}>
                                    {starredOnly ? 'STARRED ONLY' : 'ALL IMAGES'}
                                </span>
                                {selectedLabelId && (
                                    <span className="px-2 py-1 rounded text-[10px] font-bold border bg-blue-900/20 border-blue-700/50 text-blue-400 uppercase">
                                        {labelStore.labels.find(l => l.id === selectedLabelId)?.name}
                                    </span>
                                )}
                            </div>
                        </div>
                    </div>
                </div>
            )}
        </div>
    );
});