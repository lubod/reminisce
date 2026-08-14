import React from "react";
import { Sparkles, Eye, Download, Check } from "lucide-react";

interface LightboxEnhancePanelProps {
    enhanceLoading: boolean;
    enhancedUrl: string | null;
    showEnhanced: boolean;
    enhanceOps: string[];
    saveState: "idle" | "saving" | "saved";
    isAdmin: boolean;
    onEnhance: (mode: string) => void;
    onToggleShowEnhanced: () => void;
    onSaveToLibrary: () => void;
    onDownload: () => void;
}

export const LightboxEnhancePanel: React.FC<LightboxEnhancePanelProps> = ({
    enhanceLoading,
    enhancedUrl,
    showEnhanced,
    enhanceOps,
    saveState,
    isAdmin,
    onEnhance,
    onToggleShowEnhanced,
    onSaveToLibrary,
    onDownload,
}) => {
    return (
        <div
            className="absolute top-16 right-4 z-50 bg-gray-900/95 border border-purple-500/40 rounded-xl p-4 shadow-2xl backdrop-blur-md w-80 text-white"
            onClick={(e) => e.stopPropagation()}
        >
            <div className="flex items-center justify-between mb-3 pb-2 border-b border-gray-700">
                <span className="font-semibold text-purple-300 flex items-center gap-2 text-sm">
                    <Sparkles className="w-4 h-4 text-purple-400" /> AI Photo Enhancement
                </span>
                {enhanceLoading && (
                    <span className="text-xs text-purple-300 animate-pulse">Processing...</span>
                )}
            </div>

            <div className="grid grid-cols-2 gap-2 mb-3">
                {[
                    { id: "auto", label: "✨ Auto Fix", desc: "Balanced pipeline" },
                    { id: "denoise", label: "🧹 Denoise", desc: "Low-light & high ISO" },
                    { id: "restore", label: "🎨 Restore Color", desc: "Faded/scanned photos" },
                    { id: "lowlight", label: "🌙 Low-Light HDR", desc: "Dark / night shots" },
                    { id: "sharpen", label: "🔍 Sharpen", desc: "Soft focus & edges" },
                    { id: "face", label: "👤 Face Enhance", desc: "Portrait clarity" },
                ].map(({ id, label, desc }) => (
                    <button
                        key={id}
                        disabled={enhanceLoading}
                        onClick={() => onEnhance(id)}
                        className="flex flex-col items-start p-2 rounded-lg bg-gray-800 hover:bg-purple-900/40 border border-gray-700 hover:border-purple-500/60 transition-all text-left disabled:opacity-50"
                    >
                        <span className="text-xs font-medium text-gray-100">{label}</span>
                        <span className="text-xs text-gray-400">{desc}</span>
                    </button>
                ))}
            </div>

            {enhancedUrl && (
                <div className="pt-2 border-t border-gray-700 space-y-2">
                    {enhanceOps.length > 0 && (
                        <div className="text-xs text-purple-300 font-mono">
                            Applied: {enhanceOps.join(", ")}
                        </div>
                    )}
                    <div className="flex gap-2">
                        <button
                            onClick={onToggleShowEnhanced}
                            className={`flex-1 py-1.5 px-2 rounded text-xs font-medium flex items-center justify-center gap-1 border transition-colors ${
                                showEnhanced
                                    ? "bg-purple-600 border-purple-400 text-white"
                                    : "bg-gray-800 border-gray-600 text-gray-300 hover:bg-gray-700"
                            }`}
                        >
                            <Eye className="w-3.5 h-3.5" />
                            {showEnhanced ? "Showing Enhanced" : "Show Original"}
                        </button>
                        <button
                            onClick={onDownload}
                            className="p-1.5 rounded bg-gray-800 hover:bg-gray-700 border border-gray-600 text-gray-300"
                            title="Download enhanced JPEG"
                        >
                            <Download className="w-3.5 h-3.5" />
                        </button>
                    </div>
                    {isAdmin && (
                        <button
                            onClick={onSaveToLibrary}
                            disabled={saveState !== "idle"}
                            className="w-full py-1.5 px-3 rounded text-xs font-medium bg-emerald-700 hover:bg-emerald-600 disabled:opacity-50 text-white flex items-center justify-center gap-1.5 transition-colors"
                        >
                            {saveState === "saved" ? (
                                <>
                                    <Check className="w-3.5 h-3.5" /> Saved to Library
                                </>
                            ) : saveState === "saving" ? (
                                "Saving..."
                            ) : (
                                "Save as New Image in Library"
                            )}
                        </button>
                    )}
                </div>
            )}
        </div>
    );
};
