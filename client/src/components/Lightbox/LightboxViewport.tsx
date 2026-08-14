import React from "react";
import type { MediaItem } from "../../stores/MediaStore";

interface LightboxViewportProps {
    selectedMedia: MediaItem;
    fullMediaUrl: string | null;
    comparisonMedia: MediaItem | null;
    comparisonMediaUrl: string | null;
    compareMode: boolean;
    showEnhanced: boolean;
    enhancedUrl: string | null;
    enhanceLoading: boolean;
    zoomStyle: React.CSSProperties;
    zoomScale: number;
    onWheel: (e: React.WheelEvent) => void;
    onMouseDown: (e: React.MouseEvent) => void;
    onMouseMove: (e: React.MouseEvent) => void;
    onMouseUp: () => void;
    onResetZoom: () => void;
}

export const LightboxViewport: React.FC<LightboxViewportProps> = ({
    selectedMedia,
    fullMediaUrl,
    comparisonMedia,
    comparisonMediaUrl,
    compareMode,
    showEnhanced,
    enhancedUrl,
    enhanceLoading,
    zoomStyle,
    zoomScale,
    onWheel,
    onMouseDown,
    onMouseMove,
    onMouseUp,
    onResetZoom,
}) => {
    const isVideo = selectedMedia.media_type === "video";
    const isComparisonVideo = comparisonMedia?.media_type === "video";

    return (
        <div
            className="flex-1 flex items-center justify-center mb-4 overflow-hidden relative select-none"
            onWheel={onWheel}
            onMouseDown={onMouseDown}
            onMouseMove={onMouseMove}
            onMouseUp={onMouseUp}
            onMouseLeave={onMouseUp}
        >
            <div className={`w-full h-full flex gap-4 ${compareMode ? "flex-row" : "flex-col"}`}>
                {/* Primary Media */}
                <div className="flex-1 flex items-center justify-center overflow-hidden relative">
                    {fullMediaUrl ? (
                        <div style={zoomStyle} className="w-full h-full flex items-center justify-center pointer-events-none">
                            {isVideo ? (
                                <video src={fullMediaUrl} className="max-w-full max-h-full object-contain pointer-events-auto" controls autoPlay />
                            ) : (
                                <img
                                    src={showEnhanced && enhancedUrl ? enhancedUrl : fullMediaUrl}
                                    alt={selectedMedia.name}
                                    className="max-w-full max-h-full object-contain"
                                />
                            )}
                        </div>
                    ) : (
                        <div className="text-white">Loading...</div>
                    )}
                    {compareMode && (
                        <div className="absolute bottom-2 left-2 bg-black bg-opacity-60 px-2 py-1 rounded text-xs text-white">Current</div>
                    )}
                </div>

                {/* Comparison Media */}
                {compareMode && (
                    <div className="flex-1 flex items-center justify-center overflow-hidden border-l border-gray-700 relative">
                        {comparisonMediaUrl ? (
                            <div style={zoomStyle} className="w-full h-full flex items-center justify-center pointer-events-none">
                                {isComparisonVideo ? (
                                    <video src={comparisonMediaUrl} className="max-w-full max-h-full object-contain pointer-events-auto" controls />
                                ) : (
                                    <img src={comparisonMediaUrl} alt={comparisonMedia?.name} className="max-w-full max-h-full object-contain" />
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

            {/* Zoom reset indicator */}
            {zoomScale > 1 && (
                <button
                    onClick={onResetZoom}
                    className="absolute bottom-4 left-1/2 transform -translate-x-1/2 bg-blue-600 text-white px-4 py-1 rounded-full text-sm shadow-lg hover:bg-blue-500 transition-colors z-50"
                >
                    Reset Zoom ({Math.round(zoomScale * 100)}%)
                </button>
            )}
        </div>
    );
};
