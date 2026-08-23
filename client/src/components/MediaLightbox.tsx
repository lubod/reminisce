import React, { useEffect, useState, useCallback, useRef } from "react";
import { observer } from "mobx-react-lite";
import { useStore } from "../stores/RootStore";
import {
    X,
    Star,
    Trash2,
    Download,
    Columns2,
    Info,
    Wand2,
} from "lucide-react";
import axios from "../api/axiosConfig";
import { logger } from "../utils/logger";
import type { Label } from "../stores/LabelStore";
import { LightboxViewport } from "./Lightbox/LightboxViewport";
import { LightboxSidebar } from "./Lightbox/LightboxSidebar";
import { LightboxEnhancePanel } from "./Lightbox/LightboxEnhancePanel";

export const MediaLightbox = observer(() => {
    const { mediaStore, labelStore, authStore } = useStore();
    const isAdmin = authStore.user?.role === "admin";
    const [activeTab, setActiveTab] = useState<"info" | "description" | "exif" | "labels">("info");
    const [mediaLabels, setMediaLabels] = useState<Label[]>([]);
    const [showNewLabelInput, setShowNewLabelInput] = useState(false);
    const [newLabelName, setNewLabelName] = useState("");
    const [showInfo, setShowInfo] = useState(true);

    // AI Enhancement state
    const [showEnhancePanel, setShowEnhancePanel] = useState(false);
    const [enhancedUrl, setEnhancedUrl] = useState<string | null>(null);
    const [enhanceLoading, setEnhanceLoading] = useState(false);
    const [showEnhanced, setShowEnhanced] = useState(false);
    const [enhanceOps, setEnhanceOps] = useState<string[]>([]);
    const [saveState, setSaveState] = useState<"idle" | "saving" | "saved">("idle");

    // Drag-to-pan state
    const isDragging = useRef(false);
    const lastMousePos = useRef({ x: 0, y: 0 });

    const selectedMedia = mediaStore.selectedMediaIndex !== null
        ? mediaStore.activeLightboxItems[mediaStore.selectedMediaIndex]
        : null;

    const comparisonMedia = (mediaStore.compareMode && mediaStore.selectedMediaIndex !== null && mediaStore.selectedMediaIndex < mediaStore.activeLightboxItems.length - 1)
        ? mediaStore.activeLightboxItems[mediaStore.selectedMediaIndex + 1]
        : null;

    const isFirstMedia = mediaStore.selectedMediaIndex === 0;
    const isLastMedia = mediaStore.selectedMediaIndex !== null && mediaStore.selectedMediaIndex === mediaStore.activeLightboxItems.length - 1;

    const handleDelete = useCallback(async () => {
        if (selectedMedia && window.confirm("Are you sure you want to delete this media?")) {
            await mediaStore.deleteMedia(selectedMedia.hash);
        }
    }, [selectedMedia, mediaStore]);

    // Keyboard navigation
    useEffect(() => {
        const handleKeyDown = (e: KeyboardEvent) => {
            const target = e.target as HTMLElement;
            if (
                target && (
                    target.tagName === "INPUT" ||
                    target.tagName === "TEXTAREA" ||
                    target.isContentEditable
                )
            ) {
                return;
            }
            if (e.key === "Escape") {
                mediaStore.closeMediaLightbox();
            } else if (e.key === "ArrowRight") {
                mediaStore.nextMedia();
            } else if (e.key === "ArrowLeft") {
                mediaStore.previousMedia();
            } else if (e.key.toLowerCase() === "c") {
                mediaStore.toggleCompareMode();
            } else if (e.key.toLowerCase() === "i") {
                setShowInfo((prev) => !prev);
            } else if (e.key.toLowerCase() === "d" && isAdmin) {
                void handleDelete();
            }
        };

        window.addEventListener("keydown", handleKeyDown);
        return () => window.removeEventListener("keydown", handleKeyDown);
    }, [mediaStore, isAdmin, handleDelete]);

    // Mouse handlers for zoom and pan
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

    // Load metadata when description or exif tab is selected
    useEffect(() => {
        if (selectedMedia && selectedMedia.media_type !== "video" && (activeTab === "description" || activeTab === "exif")) {
            if (!mediaStore.imageMetadata || (mediaStore.imageMetadata && selectedMedia.hash !== mediaStore.lastLoadedMetadataHash)) {
                void mediaStore.loadImageMetadata(selectedMedia.hash);
            }
        }
    }, [activeTab, selectedMedia, mediaStore]);

    // Reset enhanced view & clear metadata when changing media
    useEffect(() => {
        mediaStore.clearImageMetadata();
        setShowNewLabelInput(false);
        setNewLabelName("");
        setMediaLabels([]);
        setShowEnhancePanel(false);
        setEnhanceLoading(false);
        setShowEnhanced(false);
        setEnhanceOps([]);
        setSaveState("idle");
        setEnhancedUrl((prev) => {
            if (prev) URL.revokeObjectURL(prev);
            return null;
        });
    }, [mediaStore.selectedMediaIndex, mediaStore]);

    // Cleanup object URL on unmount
    useEffect(() => {
        return () => {
            if (enhancedUrl) {
                URL.revokeObjectURL(enhancedUrl);
            }
        };
    }, [enhancedUrl]);

    // Load media labels
    const loadMediaLabels = useCallback(async () => {
        if (selectedMedia) {
            let labels: Label[];
            if (selectedMedia.media_type === "video") {
                labels = await labelStore.getVideoLabels(selectedMedia.hash);
            } else {
                labels = await labelStore.getImageLabels(selectedMedia.hash);
            }
            setMediaLabels(labels);
        }
    }, [selectedMedia, labelStore]);

    useEffect(() => {
        if (activeTab === "labels" && selectedMedia) {
            void labelStore.fetchLabels();
            void loadMediaLabels();
        }
    }, [activeTab, selectedMedia, labelStore, loadMediaLabels]);

    const handleAddLabel = async (labelId: number) => {
        if (selectedMedia) {
            if (selectedMedia.media_type === "video") {
                await labelStore.addVideoLabel(selectedMedia.hash, labelId);
            } else {
                await labelStore.addImageLabel(selectedMedia.hash, labelId);
            }
            await loadMediaLabels();
        }
    };

    const handleRemoveLabel = async (labelId: number) => {
        if (selectedMedia) {
            if (selectedMedia.media_type === "video") {
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
            if (selectedMedia.media_type === "video") {
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
                { responseType: "blob" }
            );
            const ops = ((response.headers["x-enhance-operations"] as string) || "")
                .split(",")
                .map((s) => s.trim())
                .filter(Boolean);
            setEnhancedUrl((prev) => {
                if (prev) URL.revokeObjectURL(prev);
                return null;
            });
            setEnhancedUrl(URL.createObjectURL(response.data));
            setEnhanceOps(ops);
            setShowEnhanced(true);
        } catch (e) {
            logger.error("Enhancement failed", e);
        } finally {
            setEnhanceLoading(false);
        }
    };

    const handleDownloadEnhanced = () => {
        if (!enhancedUrl || !selectedMedia) return;
        const a = document.createElement("a");
        a.href = enhancedUrl;
        a.download = selectedMedia.name.replace(/\.[^.]+$/, "") + "_enhanced.jpg";
        a.click();
    };

    const handleSaveToLibrary = async () => {
        if (!enhancedUrl || !selectedMedia || saveState !== "idle") return;
        setSaveState("saving");
        try {
            const blob = await fetch(enhancedUrl).then((r) => r.blob());
            const base64 = await new Promise<string>((resolve, reject) => {
                const reader = new FileReader();
                reader.onloadend = () => {
                    const result = reader.result as string;
                    resolve(result.split(",")[1]);
                };
                reader.onerror = reject;
                reader.readAsDataURL(blob);
            });
            await axios.post(`/image/${selectedMedia.hash}/save-enhanced`, { image: base64 });
            setSaveState("saved");
        } catch (e) {
            logger.error("Save to library failed", e);
            setSaveState("idle");
        }
    };

    if (!selectedMedia) return null;

    const isVideo = selectedMedia.media_type === "video";
    const zoomStyle = {
        transform: `scale(${mediaStore.zoomScale}) translate(${mediaStore.zoomOffset.x / mediaStore.zoomScale}px, ${mediaStore.zoomOffset.y / mediaStore.zoomScale}px)`,
        cursor: mediaStore.zoomScale > 1 ? "grab" : "default",
        transition: isDragging.current ? "none" : "transform 0.1s ease-out",
    };

    return (
        <div
            className="fixed inset-0 z-50 bg-black bg-opacity-95 flex items-center justify-center"
            onClick={() => mediaStore.closeMediaLightbox()}
        >
            {/* Toolbar */}
            <div className="absolute top-4 right-4 flex gap-2 z-50">
                <button
                    className={`p-2 bg-black bg-opacity-50 rounded hover:bg-opacity-70 transition-colors ${showInfo ? "ring-2 ring-blue-500" : ""}`}
                    onClick={(e) => {
                        e.stopPropagation();
                        setShowInfo(!showInfo);
                    }}
                    title="Toggle Information (I)"
                >
                    <Info size={24} className="text-white" />
                </button>
                <button
                    className={`p-2 bg-black bg-opacity-50 rounded hover:bg-opacity-70 transition-colors ${mediaStore.compareMode ? "ring-2 ring-blue-500" : ""}`}
                    onClick={(e) => {
                        e.stopPropagation();
                        mediaStore.toggleCompareMode();
                    }}
                    title="Toggle Compare Mode (C)"
                >
                    <Columns2 size={24} className="text-white" />
                </button>
                {!isVideo && (
                    <button
                        className={`p-2 bg-black bg-opacity-50 rounded hover:bg-opacity-70 transition-colors ${showEnhancePanel ? "ring-2 ring-purple-500 text-purple-400" : "text-white"}`}
                        onClick={(e) => {
                            e.stopPropagation();
                            setShowEnhancePanel(!showEnhancePanel);
                        }}
                        title="AI Photo Enhancement"
                    >
                        <Wand2 size={24} />
                    </button>
                )}
                <button
                    className="p-2 bg-black bg-opacity-50 rounded hover:bg-opacity-70 text-white"
                    onClick={(e) => {
                        e.stopPropagation();
                        void mediaStore.toggleStarMedia(selectedMedia.hash, selectedMedia.device_id);
                    }}
                    title="Star / Unstar"
                >
                    <Star
                        size={24}
                        className={selectedMedia.starred ? "fill-yellow-400 text-yellow-400" : "text-white"}
                    />
                </button>
                <a
                    href={mediaStore.fullMediaUrl || undefined}
                    download={selectedMedia.name}
                    className="p-2 bg-black bg-opacity-50 rounded hover:bg-opacity-70 text-white"
                    onClick={(e) => e.stopPropagation()}
                    title="Download Original"
                >
                    <Download size={24} />
                </a>
                {isAdmin && (
                    <button
                        className="p-2 bg-black bg-opacity-50 rounded hover:bg-opacity-70 text-red-400 hover:text-red-300"
                        onClick={(e) => {
                            e.stopPropagation();
                            void handleDelete();
                        }}
                        title="Delete (D)"
                    >
                        <Trash2 size={24} />
                    </button>
                )}
                <button
                    className="p-2 bg-black bg-opacity-50 rounded hover:bg-opacity-70 text-white"
                    onClick={(e) => {
                        e.stopPropagation();
                        mediaStore.closeMediaLightbox();
                    }}
                    title="Close (ESC)"
                >
                    <X size={24} />
                </button>
            </div>

            {/* AI Enhancement Panel */}
            {showEnhancePanel && !isVideo && (
                <LightboxEnhancePanel
                    enhanceLoading={enhanceLoading}
                    enhancedUrl={enhancedUrl}
                    showEnhanced={showEnhanced}
                    enhanceOps={enhanceOps}
                    saveState={saveState}
                    isAdmin={isAdmin}
                    onEnhance={handleEnhance}
                    onToggleShowEnhanced={() => setShowEnhanced((v) => !v)}
                    onSaveToLibrary={handleSaveToLibrary}
                    onDownload={handleDownloadEnhanced}
                />
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

            {/* Main Content Area */}
            <div
                className="w-full h-full flex flex-col p-4"
                onClick={(e) => e.stopPropagation()}
            >
                <LightboxViewport
                    selectedMedia={selectedMedia}
                    fullMediaUrl={mediaStore.fullMediaUrl}
                    comparisonMedia={comparisonMedia}
                    comparisonMediaUrl={mediaStore.comparisonMediaUrl}
                    compareMode={mediaStore.compareMode}
                    showEnhanced={showEnhanced}
                    enhancedUrl={enhancedUrl}
                    enhanceLoading={enhanceLoading}
                    zoomStyle={zoomStyle}
                    zoomScale={mediaStore.zoomScale}
                    onWheel={handleWheel}
                    onMouseDown={handleMouseDown}
                    onMouseMove={handleMouseMove}
                    onMouseUp={handleMouseUp}
                    onResetZoom={() => mediaStore.resetZoom()}
                />

                {!mediaStore.compareMode && showInfo && (
                    <LightboxSidebar
                        selectedMedia={selectedMedia}
                        metadata={mediaStore.imageMetadata}
                        activeTab={activeTab}
                        setActiveTab={setActiveTab}
                        mediaLabels={mediaLabels}
                        availableLabels={labelStore.labels}
                        showNewLabelInput={showNewLabelInput}
                        setShowNewLabelInput={setShowNewLabelInput}
                        newLabelName={newLabelName}
                        setNewLabelName={setNewLabelName}
                        handleAddLabel={handleAddLabel}
                        handleRemoveLabel={handleRemoveLabel}
                        handleCreateAndAddLabel={handleCreateAndAddLabel}
                    />
                )}
            </div>
        </div>
    );
});
