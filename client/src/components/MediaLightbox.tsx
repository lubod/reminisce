import { observer } from "mobx-react-lite";
import { useStore } from "../stores/RootStore";
import { useEffect, useState, useCallback, useRef } from "react";
import axios from "../api/axiosConfig";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { Star, Tag, X, Plus, Columns2, Trash2, Info, Wand2, Download, Save } from "lucide-react";
import type { Label } from "../stores/LabelStore";

export const MediaLightbox = observer(() => {
    const { mediaStore, labelStore, authStore } = useStore();
    const isAdmin = authStore.user?.role === "admin";
    const [activeTab, setActiveTab] = useState<'info' | 'description' | 'exif' | 'labels'>('info');
    const [mediaLabels, setMediaLabels] = useState<Label[]>([]);
    const [showNewLabelInput, setShowNewLabelInput] = useState(false);
    const [newLabelName, setNewLabelName] = useState("");
    const [showInfo, setShowInfo] = useState(true);

    // Enhance state
    const [showEnhancePanel, setShowEnhancePanel] = useState(false);
    const [enhancedUrl, setEnhancedUrl] = useState<string | null>(null);
    const [enhanceLoading, setEnhanceLoading] = useState(false);
    const [showEnhanced, setShowEnhanced] = useState(false);
    const [enhanceOps, setEnhanceOps] = useState<string[]>([]);
    const [saveState, setSaveState] = useState<'idle' | 'saving' | 'saved'>('idle');

    // For panning
    const isDragging = useRef(false);
    const lastMousePos = useRef({ x: 0, y: 0 });

    const selectedMedia = mediaStore.selectedMediaIndex !== null
        ? mediaStore.activeLightboxItems[mediaStore.selectedMediaIndex]
        : null;
    
    const comparisonMedia = (mediaStore.compareMode && mediaStore.selectedMediaIndex !== null && mediaStore.selectedMediaIndex < mediaStore.activeLightboxItems.length - 1)
        ? mediaStore.activeLightboxItems[mediaStore.selectedMediaIndex + 1]
        : null;

    const isFirstMedia = mediaStore.selectedMediaIndex === 0;
    const isLastMedia = mediaStore.selectedMediaIndex === mediaStore.activeLightboxItems.length - 1;

    useEffect(() => {
        const handleKeyDown = (e: KeyboardEvent) => {
            if (e.key === "Escape") {
                mediaStore.closeMediaLightbox();
            } else if (e.key === "ArrowRight") {
                mediaStore.nextMedia();
            } else if (e.key === "ArrowLeft") {
                mediaStore.previousMedia();
            } else if (e.key.toLowerCase() === "c") {
                mediaStore.toggleCompareMode();
            } else if (e.key.toLowerCase() === "i") {
                setShowInfo(prev => !prev);
            } else if (e.key.toLowerCase() === "d" && isAdmin) {
                handleDelete();
            }
        };

        window.addEventListener("keydown", handleKeyDown);
        return () => window.removeEventListener("keydown", handleKeyDown);
    }, [mediaStore, isAdmin]);

    const handleDelete = async () => {
        if (selectedMedia && window.confirm("Are you sure you want to delete this media?")) {
            await mediaStore.deleteMedia(selectedMedia.hash);
        }
    };

    // Pan & Zoom Handlers
    const handleWheel = (e: React.WheelEvent) => {
        e.preventDefault();
        const delta = e.deltaY > 0 ? 0.9 : 1.1;
        mediaStore.setZoomScale(mediaStore.zoomScale * delta);
    };

    const handleMouseDown = (e: React.MouseEvent) => {
        if (mediaStore.zoomScale > 1) {
            isDragging.current = true;
            lastMousePos.current = { x: e.clientX, y: e.clientY };
        }
    };

    const handleMouseMove = (e: React.MouseEvent) => {
        if (isDragging.current) {
            const dx = e.clientX - lastMousePos.current.x;
            const dy = e.clientY - lastMousePos.current.y;
            mediaStore.setZoomOffset(mediaStore.zoomOffset.x + dx, mediaStore.zoomOffset.y + dy);
            lastMousePos.current = { x: e.clientX, y: e.clientY };
        }
    };

    const handleMouseUp = () => {
        isDragging.current = false;
    };

    // Load metadata when description or exif tab is selected (only for images)
    useEffect(() => {
        if (selectedMedia && selectedMedia.media_type !== 'video' && (activeTab === 'description' || activeTab === 'exif')) {
            // Only load if we don't have metadata OR if the metadata is for a different image
            if (!mediaStore.imageMetadata ||
                (mediaStore.imageMetadata && selectedMedia.hash !== mediaStore.lastLoadedMetadataHash)) {
                mediaStore.loadImageMetadata(selectedMedia.hash);
            }
        }
    }, [activeTab, selectedMedia, mediaStore]);

    // Clear metadata and enhance state when media changes
    useEffect(() => {
        mediaStore.clearImageMetadata();
        setShowNewLabelInput(false);
        setNewLabelName("");
        setMediaLabels([]);
        setShowEnhancePanel(false);
        setEnhanceLoading(false);
        setShowEnhanced(false);
        setEnhanceOps([]);
        setSaveState('idle');
        setEnhancedUrl(prev => {
            if (prev) URL.revokeObjectURL(prev);
            return null;
        });
    }, [mediaStore.selectedMediaIndex, mediaStore]);

    const loadMediaLabels = useCallback(async () => {
        if (selectedMedia) {
            let labels: Label[];
            if (selectedMedia.media_type === 'video') {
                labels = await labelStore.getVideoLabels(selectedMedia.hash);
            } else {
                labels = await labelStore.getImageLabels(selectedMedia.hash);
            }
            setMediaLabels(labels);
        }
    }, [selectedMedia, labelStore]);

    // Load labels when labels tab is selected
    useEffect(() => {
        if (activeTab === 'labels' && selectedMedia) {
            labelStore.fetchLabels();
            loadMediaLabels();
        }
    }, [activeTab, selectedMedia, labelStore, loadMediaLabels]);

    const handleAddLabel = async (labelId: number) => {
        if (selectedMedia) {
            if (selectedMedia.media_type === 'video') {
                await labelStore.addVideoLabel(selectedMedia.hash, labelId);
            } else {
                await labelStore.addImageLabel(selectedMedia.hash, labelId);
            }
            await loadMediaLabels();
        }
    };

    const handleRemoveLabel = async (labelId: number) => {
        if (selectedMedia) {
            if (selectedMedia.media_type === 'video') {
                await labelStore.removeVideoLabel(selectedMedia.hash, labelId);
            } else {
                await labelStore.removeImageLabel(selectedMedia.hash, labelId);
            }
            await loadMediaLabels();
        }
    };

    const handleCreateAndAddLabel = async () => {
        if (newLabelName.trim() && selectedMedia) {
            const label = await labelStore.createLabel(newLabelName.trim());
            if (selectedMedia.media_type === 'video') {
                await labelStore.addVideoLabel(selectedMedia.hash, label.id);
            } else {
                await labelStore.addImageLabel(selectedMedia.hash, label.id);
            }
            await loadMediaLabels();
            setNewLabelName("");
            setShowNewLabelInput(false);
        }
    };

    const handleEnhance = async (mode: string) => {
        if (!selectedMedia) return;
        setEnhanceLoading(true);
        setShowEnhancePanel(false);
        try {
            const response = await axios.post(
                `/image/${selectedMedia.hash}/enhance?mode=${mode}`,
                null,
                { responseType: 'blob' }
            );
            const ops = ((response.headers['x-enhance-operations'] as string) || '')
                .split(',').filter(Boolean);
            setEnhancedUrl(prev => { if (prev) URL.revokeObjectURL(prev); return null; });
            setEnhancedUrl(URL.createObjectURL(response.data));
            setEnhanceOps(ops);
            setShowEnhanced(true);
        } catch (e) {
            console.error('Enhancement failed', e);
        } finally {
            setEnhanceLoading(false);
        }
    };

    const handleDownloadEnhanced = () => {
        if (!enhancedUrl || !selectedMedia) return;
        const a = document.createElement('a');
        a.href = enhancedUrl;
        a.download = selectedMedia.name.replace(/\.[^.]+$/, '') + '_enhanced.jpg';
        a.click();
    };

    const handleSaveToLibrary = async () => {
        if (!enhancedUrl || !selectedMedia || saveState !== 'idle') return;
        setSaveState('saving');
        try {
            // Fetch the blob and convert to base64
            const blob = await fetch(enhancedUrl).then(r => r.blob());
            const base64 = await new Promise<string>((resolve, reject) => {
                const reader = new FileReader();
                reader.onloadend = () => {
                    const result = reader.result as string;
                    resolve(result.split(',')[1]); // strip data URI prefix
                };
                reader.onerror = reject;
                reader.readAsDataURL(blob);
            });
            await axios.post(`/image/${selectedMedia.hash}/save-enhanced`, { image: base64 });
            setSaveState('saved');
        } catch (e) {
            console.error('Save to library failed', e);
            setSaveState('idle');
        }
    };

    if (!selectedMedia) return null;

    const formatFileSize = (bytes: number): string => {
        if (bytes >= 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
        if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
        if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)} KB`;
        return `${bytes} B`;
    };

    const formatDate = (dateString: string) => {
        const date = new Date(dateString);
        return date.toLocaleString('en-US', {
            year: 'numeric',
            month: 'long',
            day: 'numeric',
            hour: '2-digit',
            minute: '2-digit'
        });
    };

    const formatExifData = (exifJson: string) => {
        try {
            const exif = JSON.parse(exifJson);
            const formatted: { [key: string]: unknown } = {};

            // Strip surrounding double-quotes that kamadak-exif adds to ASCII string display values
            // e.g. stored '"HONOR"' (string with embedded quote chars) → 'HONOR'
            const strip = (v: unknown): string =>
                typeof v === 'string' ? v.replace(/^"|"$/g, '').trim() : String(v);

            // Extract leading number from strings like "6.67 mm", "27 mm", "3072 pixels"
            const leadNum = (v: unknown): number | null => {
                const n = parseFloat(String(v));
                return isNaN(n) ? null : n;
            };

            // Camera info
            if (exif.Make) formatted['Camera Make'] = strip(exif.Make);
            if (exif.Model) formatted['Camera Model'] = strip(exif.Model);
            if (exif.LensModel) formatted['Lens'] = strip(exif.LensModel);

            // Capture settings
            if (exif.ExposureTime) {
                if (typeof exif.ExposureTime === 'number') {
                    // Numeric value (e.g. 0.001587)
                    formatted['Shutter Speed'] = exif.ExposureTime >= 1
                        ? `${exif.ExposureTime}s`
                        : `1/${Math.round(1 / exif.ExposureTime)}s`;
                } else {
                    // Pre-formatted string like "1/630 s" — strip trailing " s"
                    formatted['Shutter Speed'] = String(exif.ExposureTime).replace(/ s$/, '');
                }
            }
            if (exif.FNumber) {
                // May already include "f/" prefix (e.g. "f/1.9") or be a bare number
                const fn_ = String(exif.FNumber);
                formatted['Aperture'] = fn_.startsWith('f/') ? fn_ : `f/${fn_}`;
            }
            // ISO: check all key names used by different EXIF sources / versions
            const isoVal = exif.ISO ?? exif.ISOSpeedRatings ?? exif.ISOSpeed ?? exif.PhotographicSensitivity;
            if (isoVal != null) formatted['ISO'] = isoVal;

            if (exif.FocalLength) {
                const n = leadNum(exif.FocalLength);
                formatted['Focal Length'] = n != null ? `${n}mm` : String(exif.FocalLength);
            }
            if (exif.FocalLengthIn35mmFilm) {
                const n = leadNum(exif.FocalLengthIn35mmFilm);
                formatted['Focal Length (35mm equiv.)'] = n != null ? `${n}mm` : String(exif.FocalLengthIn35mmFilm);
            }

            // Image info — prefer PixelXDimension/PixelYDimension; fall back to
            // ImageWidth / ImageLength (EXIF standard name for height) or ImageHeight
            const imgW = leadNum(exif.PixelXDimension ?? exif.ImageWidth);
            const imgH = leadNum(exif.PixelYDimension ?? exif.ImageHeight ?? exif.ImageLength);
            if (imgW && imgH) formatted['Resolution'] = `${imgW} × ${imgH}`;

            if (exif.Orientation) {
                const orientations: { [key: number]: string } = {
                    1: 'Normal', 2: 'Flipped horizontal', 3: 'Rotated 180°',
                    4: 'Flipped vertical', 5: 'Rotated 90° CCW + flip',
                    6: 'Rotated 90° CW', 7: 'Rotated 90° CW + flip', 8: 'Rotated 90° CCW'
                };
                // kamadak-exif stores Orientation as a verbose display string, e.g.
                // "row 0 at right and column 0 at top" — map those to numeric values.
                const orientationStrings: { [key: string]: number } = {
                    'row 0 at top and column 0 at left': 1,
                    'row 0 at top and column 0 at right': 2,
                    'row 0 at bottom and column 0 at right': 3,
                    'row 0 at bottom and column 0 at left': 4,
                    'row 0 at left and column 0 at top': 5,
                    'row 0 at right and column 0 at top': 6,
                    'row 0 at right and column 0 at bottom': 7,
                    'row 0 at left and column 0 at bottom': 8,
                };
                const ori = typeof exif.Orientation === 'number'
                    ? exif.Orientation
                    : (orientationStrings[String(exif.Orientation)] ?? parseInt(exif.Orientation, 10));
                formatted['Orientation'] = orientations[ori] || String(exif.Orientation);
            }

            // Date/time
            if (exif.DateTime) formatted['Date Taken'] = exif.DateTime;
            if (exif.DateTimeOriginal) formatted['Date Original'] = exif.DateTimeOriginal;

            // GPS
            if (exif.GPSLatitude && exif.GPSLongitude) {
                formatted['GPS'] = `${exif.GPSLatitude}, ${exif.GPSLongitude}`;
            }

            // Other
            if (exif.Flash) formatted['Flash'] = exif.Flash;
            if (exif.WhiteBalance != null) {
                if (typeof exif.WhiteBalance === 'number') {
                    formatted['White Balance'] = exif.WhiteBalance === 0 ? 'Auto' : 'Manual';
                } else {
                    formatted['White Balance'] = strip(exif.WhiteBalance);
                }
            }
            if (exif.ExposureMode != null) {
                const modes: { [key: number]: string } = { 0: 'Auto', 1: 'Manual', 2: 'Auto bracket' };
                if (typeof exif.ExposureMode === 'number') {
                    formatted['Exposure Mode'] = modes[exif.ExposureMode] ?? String(exif.ExposureMode);
                } else {
                    formatted['Exposure Mode'] = strip(exif.ExposureMode);
                }
            }
            if (exif.MeteringMode != null) {
                const modes: { [key: number]: string } = {
                    1: 'Average', 2: 'Center-weighted', 3: 'Spot', 5: 'Pattern', 6: 'Partial'
                };
                if (typeof exif.MeteringMode === 'number') {
                    formatted['Metering Mode'] = modes[exif.MeteringMode] ?? String(exif.MeteringMode);
                } else {
                    formatted['Metering Mode'] = strip(exif.MeteringMode);
                }
            }

            return { formatted, raw: exif };
        } catch {
            return { formatted: {}, raw: {} };
        }
    };

    const isVideo = selectedMedia.media_type === 'video';
    const isComparisonVideo = comparisonMedia?.media_type === 'video';

    const zoomStyle = {
        transform: `scale(${mediaStore.zoomScale}) translate(${mediaStore.zoomOffset.x / mediaStore.zoomScale}px, ${mediaStore.zoomOffset.y / mediaStore.zoomScale}px)`,
        cursor: mediaStore.zoomScale > 1 ? 'grab' : 'default',
        transition: isDragging.current ? 'none' : 'transform 0.1s ease-out'
    };

    return (
        <div
            className="fixed inset-0 z-50 bg-black bg-opacity-95 flex items-center justify-center"
            onClick={() => mediaStore.closeMediaLightbox()}
        >
            {/* Toolbar */}
            <div className="absolute top-4 right-4 flex gap-2 z-50">
                <button
                    className={`p-2 bg-black bg-opacity-50 rounded hover:bg-opacity-70 transition-colors ${showInfo ? 'ring-2 ring-blue-500' : ''}`}
                    onClick={(e) => {
                        e.stopPropagation();
                        setShowInfo(!showInfo);
                    }}
                    title="Toggle Information (I)"
                >
                    <Info size={24} className="text-white" />
                </button>
                <button
                    className={`p-2 bg-black bg-opacity-50 rounded hover:bg-opacity-70 transition-colors ${mediaStore.compareMode ? 'ring-2 ring-blue-500' : ''}`}
                    onClick={(e) => {
                        e.stopPropagation();
                        mediaStore.toggleCompareMode();
                    }}
                    title="Side-by-side comparison (C)"
                >
                    <Columns2 size={24} className="text-white" />
                </button>
                {isAdmin && (
                    <button
                        className="p-2 bg-black bg-opacity-50 rounded hover:bg-red-900/60 transition-colors"
                        onClick={(e) => {
                            e.stopPropagation();
                            handleDelete();
                        }}
                        title="Delete media (D)"
                    >
                        <Trash2 size={24} className="text-white hover:text-red-400" />
                    </button>
                )}
                <button
                    className="p-2 bg-black bg-opacity-50 rounded hover:bg-opacity-70 transition-colors"
                    onClick={(e) => {
                        e.stopPropagation();
                        mediaStore.toggleStarMedia(selectedMedia.hash);
                    }}
                    aria-label={selectedMedia.starred ? "Unstar media" : "Star media"}
                >
                    <Star
                        size={24}
                        className={selectedMedia.starred ? 'fill-yellow-400 text-yellow-400' : 'text-white'}
                    />
                </button>
                {!isVideo && (
                    <button
                        className={`p-2 bg-black bg-opacity-50 rounded hover:bg-opacity-70 transition-colors ${showEnhancePanel || enhancedUrl ? 'ring-2 ring-purple-500' : ''}`}
                        onClick={(e) => {
                            e.stopPropagation();
                            if (enhancedUrl) {
                                setShowEnhanced(s => !s);
                            } else {
                                setShowEnhancePanel(p => !p);
                            }
                        }}
                        title={enhancedUrl ? "Toggle original / enhanced" : "Enhance photo"}
                    >
                        <Wand2
                            size={24}
                            className={enhancedUrl || showEnhancePanel ? 'text-purple-400' : 'text-white'}
                        />
                    </button>
                )}
                <button
                    className="p-2 bg-black bg-opacity-50 rounded hover:bg-opacity-70 text-white text-3xl leading-none transition-colors"
                    onClick={() => mediaStore.closeMediaLightbox()}
                >
                    &times;
                </button>
            </div>

            {/* Enhance mode panel */}
            {showEnhancePanel && !isVideo && (
                <div
                    className="absolute top-16 right-4 z-50 bg-gray-900 border border-gray-700 rounded-xl p-3 shadow-xl animate-in fade-in slide-in-from-top-2 duration-200"
                    onClick={(e) => e.stopPropagation()}
                >
                    <div className="text-xs text-gray-400 mb-2 font-medium">Enhancement mode</div>
                    <div className="grid grid-cols-2 gap-2">
                        {([
                            { mode: 'auto',     label: 'Auto',         desc: 'Smart detect' },
                            { mode: 'exposure', label: 'Exposure',     desc: 'Fix brightness' },
                            { mode: 'restore',  label: 'Restore',      desc: 'Old / faded' },
                            { mode: 'all',      label: 'Full',         desc: 'Everything' },
                        ] as const).map(({ mode, label, desc }) => (
                            <button
                                key={mode}
                                onClick={() => handleEnhance(mode)}
                                className="flex flex-col items-start px-3 py-2 bg-gray-800 hover:bg-purple-900/50 border border-gray-700 hover:border-purple-500 rounded-lg transition-colors text-left"
                            >
                                <span className="text-sm text-white font-medium">{label}</span>
                                <span className="text-xs text-gray-400">{desc}</span>
                            </button>
                        ))}
                    </div>
                </div>
            )}

            {/* Previous button */}
            {!isFirstMedia && (
                <button
                    className="absolute left-4 text-white text-5xl hover:text-gray-300 z-50 bg-black bg-opacity-20 rounded-full w-12 h-12 flex items-center justify-center"
                    onClick={(e) => {
                        e.stopPropagation();
                        mediaStore.previousMedia();
                    }}
                >
                    &#8249;
                </button>
            )}

            {/* Next button */}
            {!isLastMedia && (
                <button
                    className="absolute right-4 text-white text-5xl hover:text-gray-300 z-50 bg-black bg-opacity-20 rounded-full w-12 h-12 flex items-center justify-center"
                    onClick={(e) => {
                        e.stopPropagation();
                        mediaStore.nextMedia();
                    }}
                >
                    &#8250;
                </button>
            )}

            {/* Content container */}
            <div
                className="w-full h-full flex flex-col p-4"
                onClick={(e) => e.stopPropagation()}
            >
                {/* Media viewport */}
                <div 
                    className="flex-1 flex items-center justify-center mb-4 overflow-hidden relative select-none"
                    onWheel={handleWheel}
                    onMouseDown={handleMouseDown}
                    onMouseMove={handleMouseMove}
                    onMouseUp={handleMouseUp}
                    onMouseLeave={handleMouseUp}
                >
                    <div className={`w-full h-full flex gap-4 ${mediaStore.compareMode ? 'flex-row' : 'flex-col'}`}>
                        {/* Primary Media */}
                        <div className="flex-1 flex items-center justify-center overflow-hidden relative">
                            {mediaStore.fullMediaUrl ? (
                                <div style={zoomStyle} className="w-full h-full flex items-center justify-center pointer-events-none">
                                    {isVideo ? (
                                        <video src={mediaStore.fullMediaUrl} className="max-w-full max-h-full object-contain pointer-events-auto" controls autoPlay />
                                    ) : (
                                        <img
                                            src={showEnhanced && enhancedUrl ? enhancedUrl : mediaStore.fullMediaUrl ?? undefined}
                                            alt={selectedMedia.name}
                                            className="max-w-full max-h-full object-contain"
                                        />
                                    )}
                                </div>
                            ) : (
                                <div className="text-white">Loading...</div>
                            )}
                            {mediaStore.compareMode && (
                                <div className="absolute bottom-2 left-2 bg-black bg-opacity-60 px-2 py-1 rounded text-xs text-white">Current</div>
                            )}
                        </div>

                        {/* Comparison Media */}
                        {mediaStore.compareMode && (
                            <div className="flex-1 flex items-center justify-center overflow-hidden border-l border-gray-700 relative">
                                {mediaStore.comparisonMediaUrl ? (
                                    <div style={zoomStyle} className="w-full h-full flex items-center justify-center pointer-events-none">
                                        {isComparisonVideo ? (
                                            <video src={mediaStore.comparisonMediaUrl} className="max-w-full max-h-full object-contain pointer-events-auto" controls />
                                        ) : (
                                            <img src={mediaStore.comparisonMediaUrl} alt={comparisonMedia?.name} className="max-w-full max-h-full object-contain" />
                                        )}
                                    </div>
                                ) : (
                                    <div className="text-gray-500 italic">No next media to compare</div>
                                )}
                                <div className="absolute bottom-2 left-2 bg-black bg-opacity-60 px-2 py-1 rounded text-xs text-white">Next</div>
                            </div>
                        )}
                    </div>
                    
                    {/* Enhancement loading overlay */}
                    {enhanceLoading && (
                        <div className="absolute inset-0 flex flex-col items-center justify-center bg-black bg-opacity-60 z-40 rounded">
                            <div className="w-10 h-10 border-4 border-purple-500 border-t-transparent rounded-full animate-spin mb-3" />
                            <div className="text-purple-300 text-sm font-medium">Enhancing photo…</div>
                        </div>
                    )}

                    {/* Enhance result bar */}
                    {enhancedUrl && !enhanceLoading && (
                        <div
                            className="absolute bottom-4 left-1/2 -translate-x-1/2 flex items-center gap-2 bg-gray-900/90 border border-gray-700 rounded-full px-2 py-1 shadow-xl z-50 animate-in fade-in duration-200"
                            onClick={(e) => e.stopPropagation()}
                        >
                            {/* Original / Enhanced toggle */}
                            <div className="flex rounded-full overflow-hidden border border-gray-600">
                                <button
                                    onClick={() => setShowEnhanced(false)}
                                    className={`px-3 py-1 text-xs font-medium transition-colors ${!showEnhanced ? 'bg-gray-600 text-white' : 'text-gray-400 hover:text-gray-200'}`}
                                >
                                    Original
                                </button>
                                <button
                                    onClick={() => setShowEnhanced(true)}
                                    className={`px-3 py-1 text-xs font-medium transition-colors ${showEnhanced ? 'bg-purple-600 text-white' : 'text-gray-400 hover:text-gray-200'}`}
                                >
                                    Enhanced
                                </button>
                            </div>

                            {/* Applied operations */}
                            {enhanceOps.length > 0 && (
                                <span className="text-xs text-gray-400 hidden sm:inline">
                                    {enhanceOps.join(' · ')}
                                </span>
                            )}

                            {/* Save to Library */}
                            <button
                                onClick={handleSaveToLibrary}
                                disabled={saveState !== 'idle'}
                                className={`flex items-center gap-1 px-2 py-1 rounded-full text-xs font-medium transition-colors ${
                                    saveState === 'saved'
                                        ? 'bg-green-700 text-white cursor-default'
                                        : saveState === 'saving'
                                        ? 'bg-gray-600 text-gray-300 cursor-wait'
                                        : 'bg-gray-700 hover:bg-gray-600 text-white'
                                }`}
                                title="Save enhanced image to your library as a new photo"
                            >
                                <Save size={12} />
                                {saveState === 'saved' ? 'Saved!' : saveState === 'saving' ? 'Saving…' : 'Save'}
                            </button>

                            {/* Download */}
                            <button
                                onClick={handleDownloadEnhanced}
                                className="p-1.5 rounded-full bg-purple-700 hover:bg-purple-600 text-white transition-colors"
                                title="Download enhanced image"
                            >
                                <Download size={14} />
                            </button>

                            {/* Dismiss */}
                            <button
                                onClick={() => {
                                    setEnhancedUrl(prev => { if (prev) URL.revokeObjectURL(prev); return null; });
                                    setShowEnhanced(false);
                                    setEnhanceOps([]);
                                }}
                                className="p-1.5 rounded-full text-gray-400 hover:text-white transition-colors"
                                title="Dismiss enhancement"
                            >
                                <X size={14} />
                            </button>
                        </div>
                    )}

                    {/* Zoom reset indicator */}
                    {mediaStore.zoomScale > 1 && (
                        <button
                            onClick={() => mediaStore.resetZoom()}
                            className="absolute bottom-4 left-1/2 transform -translate-x-1/2 bg-blue-600 text-white px-4 py-1 rounded-full text-sm shadow-lg hover:bg-blue-500 transition-colors z-50"
                        >
                            Reset Zoom ({Math.round(mediaStore.zoomScale * 100)}%)
                        </button>
                    )}
                </div>

                {/* Info panel */}
                {!mediaStore.compareMode && showInfo && (
                    <div className="bg-gray-900 bg-opacity-80 text-white p-4 rounded h-80 flex flex-col animate-in fade-in slide-in-from-bottom-4 duration-300">
                        {/* Title */}
                        <div className="mb-3">
                            <div className="text-lg font-semibold truncate">{selectedMedia.name}</div>
                        </div>

                        {/* Tabs */}
                        <div className="flex gap-2 mb-4 border-b border-gray-700">
                            <button
                                onClick={() => setActiveTab('info')}
                                className={`px-4 py-2 text-sm font-medium transition-colors ${
                                    activeTab === 'info'
                                        ? 'text-white border-b-2 border-blue-500'
                                        : 'text-gray-400 hover:text-gray-200'
                                }`}
                            >
                                Info
                            </button>
                            {!isVideo && (
                                <>
                                    <button
                                        onClick={() => setActiveTab('description')}
                                        className={`px-4 py-2 text-sm font-medium transition-colors ${
                                            activeTab === 'description'
                                                ? 'text-white border-b-2 border-blue-500'
                                                : 'text-gray-400 hover:text-gray-200'
                                        }`}
                                    >
                                        Description
                                    </button>
                                    <button
                                        onClick={() => setActiveTab('exif')}
                                        className={`px-4 py-2 text-sm font-medium transition-colors ${
                                            activeTab === 'exif'
                                                ? 'text-white border-b-2 border-blue-500'
                                                : 'text-gray-400 hover:text-gray-200'
                                        }`}
                                    >
                                        EXIF
                                    </button>
                                </>
                            )}
                            <button
                                onClick={() => setActiveTab('labels')}
                                className={`px-4 py-2 text-sm font-medium transition-colors flex items-center gap-1 ${
                                    activeTab === 'labels'
                                        ? 'text-white border-b-2 border-blue-500'
                                        : 'text-gray-400 hover:text-gray-200'
                                }`}
                            >
                                <Tag size={16} />
                                Labels
                            </button>
                        </div>

                        {/* Tab Content - scrollable */}
                        <div className="overflow-y-auto flex-1" onWheel={(e) => e.stopPropagation()}>
                            {activeTab === 'info' && (
                                <div className="grid grid-cols-1 md:grid-cols-2 gap-4 text-sm">
                                    <div>
                                        <div className="text-gray-400 text-xs mb-1">Date & Time</div>
                                        <div>{formatDate(selectedMedia.created_at)}</div>
                                    </div>
                                    <div>
                                        <div className="text-gray-400 text-xs mb-1">Location</div>
                                        <div className="truncate">{selectedMedia.place || "Unknown"}</div>
                                    </div>
                                    {selectedMedia.device_id && (
                                        <div className="md:col-span-2">
                                            <div className="text-gray-400 text-xs mb-1">Device ID</div>
                                            <div className="font-mono text-xs">{selectedMedia.device_id}</div>
                                        </div>
                                    )}
                                    <div className="md:col-span-2">
                                        <div className="text-gray-400 text-xs mb-1">Type</div>
                                        <div className="capitalize">{selectedMedia.media_type || (isVideo ? 'Video' : 'Image')}</div>
                                    </div>
                                    {selectedMedia.file_size_bytes != null && (
                                        <div>
                                            <div className="text-gray-400 text-xs mb-1">File Size</div>
                                            <div>{formatFileSize(selectedMedia.file_size_bytes)}</div>
                                        </div>
                                    )}
                                </div>
                            )}

                            {activeTab === 'description' && !isVideo && (
                                <div className="text-sm">
                                    {mediaStore.imageMetadata ? (
                                        <>
                                            {mediaStore.imageMetadata.description ? (
                                                <div>
                                                    <div className="text-gray-400 text-xs mb-2">AI Description</div>
                                                    <div className="text-white prose prose-invert prose-sm max-w-none
                                                        prose-headings:text-white prose-headings:font-semibold
                                                        prose-p:text-gray-200 prose-p:leading-relaxed
                                                        prose-a:text-blue-400 prose-a:no-underline hover:prose-a:underline
                                                        prose-strong:text-white prose-strong:font-semibold
                                                        prose-code:text-blue-300 prose-code:bg-gray-800 prose-code:px-1 prose-code:py-0.5 prose-code:rounded
                                                        prose-pre:bg-gray-800 prose-pre:text-gray-200
                                                        prose-ul:text-gray-200 prose-ol:text-gray-200
                                                        prose-li:text-gray-200 prose-li:marker:text-gray-400
                                                        prose-blockquote:text-gray-300 prose-blockquote:border-gray-600
                                                        prose-hr:border-gray-600">
                                                        <ReactMarkdown remarkPlugins={[remarkGfm]}>
                                                            {mediaStore.imageMetadata.description}
                                                        </ReactMarkdown>
                                                    </div>
                                                </div>
                                            ) : (
                                                <div className="text-gray-400 italic">No description available.</div>
                                            )}
                                        </>
                                    ) : (
                                        <div className="text-gray-400">Loading metadata...</div>
                                    )}
                                </div>
                            )}

                            {activeTab === 'exif' && !isVideo && (
                                <div className="text-sm">
                                    {mediaStore.imageMetadata ? (
                                        <>
                                            {mediaStore.imageMetadata.exif ? (() => {
                                                const { formatted, raw } = formatExifData(mediaStore.imageMetadata.exif);
                                                const hasFormattedData = Object.keys(formatted).length > 0;

                                                return (
                                                    <div>
                                                        {hasFormattedData ? (
                                                            <>
                                                                <div className="grid grid-cols-1 md:grid-cols-2 gap-3 mb-3">
                                                                    {Object.entries(formatted).map(([key, value]) => (
                                                                        <div key={key} className="text-xs">
                                                                            <div className="text-gray-500 mb-0.5">{key}</div>
                                                                            <div className="text-gray-200">{String(value)}</div>
                                                                        </div>
                                                                    ))}
                                                                </div>
                                                                <details className="mt-3">
                                                                    <summary className="text-xs text-gray-400 cursor-pointer hover:text-gray-300">
                                                                        Show raw EXIF data
                                                                    </summary>
                                                                    <pre className="text-xs bg-gray-800 p-2 rounded overflow-auto max-h-40 mt-2">
                                                                        {JSON.stringify(raw, null, 2)}
                                                                    </pre>
                                                                </details>
                                                            </>
                                                        ) : (
                                                            <pre className="text-xs bg-gray-800 p-2 rounded overflow-auto max-h-40">
                                                                {JSON.stringify(raw, null, 2)}
                                                            </pre>
                                                        )}
                                                    </div>
                                                );
                                            })() : (
                                                <div className="text-gray-400 italic">No EXIF data available.</div>
                                            )}
                                        </>
                                    ) : (
                                        <div className="text-gray-400">Loading metadata...</div>
                                    )}
                                </div>
                            )}

                            {activeTab === 'labels' && (
                                <div className="text-sm space-y-4">
                                    {/* Current Labels */}
                                    <div>
                                        <div className="text-gray-400 text-xs mb-2">Current Labels</div>
                                        {mediaLabels.length > 0 ? (
                                            <div className="flex flex-wrap gap-2">
                                                {mediaLabels.map((label) => (
                                                    <span
                                                        key={label.id}
                                                        className="inline-flex items-center gap-1 px-3 py-1 rounded-full text-sm"
                                                        style={{ backgroundColor: label.color + '20', color: label.color }}
                                                    >
                                                        {label.name}
                                                        <button
                                                            onClick={() => handleRemoveLabel(label.id)}
                                                            className="hover:bg-black hover:bg-opacity-20 rounded-full p-0.5"
                                                        >
                                                            <X size={14} />
                                                        </button>
                                                    </span>
                                                ))}
                                            </div>
                                        ) : (
                                            <div className="text-gray-500 italic text-xs">No labels yet</div>
                                        )}
                                    </div>

                                    {/* Available Labels */}
                                    <div>
                                        <div className="text-gray-400 text-xs mb-2">Add Label</div>
                                        <div className="flex flex-wrap gap-2">
                                            {labelStore.labels
                                                .filter(l => !mediaLabels.find(ml => ml.id === l.id))
                                                .map((label) => (
                                                    <button
                                                        key={label.id}
                                                        onClick={() => handleAddLabel(label.id)}
                                                        className="px-3 py-1 rounded-full text-sm hover:opacity-80 transition-opacity"
                                                        style={{ backgroundColor: label.color + '20', color: label.color }}
                                                    >
                                                        + {label.name}
                                                    </button>
                                                ))}

                                            {/* New Label Button */}
                                            {!showNewLabelInput ? (
                                                <button
                                                    onClick={() => setShowNewLabelInput(true)}
                                                    className="px-3 py-1 rounded-full text-sm bg-gray-700 text-gray-300 hover:bg-gray-600 flex items-center gap-1"
                                                >
                                                    <Plus size={14} />
                                                    New Label
                                                </button>
                                            ) : (
                                                <div className="flex items-center gap-2">
                                                    <input
                                                        type="text"
                                                        value={newLabelName}
                                                        onChange={(e) => setNewLabelName(e.target.value)}
                                                        onKeyPress={(e) => {
                                                            if (e.key === 'Enter') {
                                                                handleCreateAndAddLabel();
                                                            } else if (e.key === 'Escape') {
                                                                setShowNewLabelInput(false);
                                                                setNewLabelName("");
                                                            }
                                                        }}
                                                        placeholder="Label name"
                                                        className="px-3 py-1 rounded-full text-sm bg-gray-700 text-white border border-gray-600 focus:outline-none focus:border-blue-500"
                                                        autoFocus
                                                    />
                                                    <button
                                                        onClick={handleCreateAndAddLabel}
                                                        className="px-2 py-1 bg-blue-600 hover:bg-blue-700 text-white rounded text-xs"
                                                    >
                                                        Add
                                                    </button>
                                                    <button
                                                        onClick={() => {
                                                            setShowNewLabelInput(false);
                                                            setNewLabelName("");
                                                        }}
                                                        className="px-2 py-1 bg-gray-600 hover:bg-gray-500 text-white rounded text-xs"
                                                    >
                                                        Cancel
                                                    </button>
                                                </div>
                                            )}
                                        </div>
                                    </div>
                                </div>
                            )}
                        </div>
                    </div>
                )}
            </div>
        </div>
    );
});

