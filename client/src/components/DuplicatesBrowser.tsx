import { useEffect, useState } from "react";
import { observer } from "mobx-react-lite";
import { useStore } from "../stores/RootStore";
import { RefreshCw, Trash2, Copy } from "lucide-react";
import { DuplicatesLightbox } from "./DuplicatesLightbox";

export const DuplicatesBrowser = observer(() => {
    const { duplicatesStore, authStore } = useStore();
    const isAdmin = authStore.user?.role === "admin";
    const token = authStore.token;
    const [lightboxState, setLightboxState] = useState<{ groupIdx: number; imageIdx: number } | null>(null);

    useEffect(() => {
        duplicatesStore.fetchDuplicates();
    }, [duplicatesStore]);

    // Close lightbox if the group it's showing disappears
    useEffect(() => {
        if (!lightboxState) return;
        if (!duplicatesStore.groups[lightboxState.groupIdx]) {
            setLightboxState(null);
        }
    }, [duplicatesStore.groups.length, lightboxState]);

    const getThumbSrc = (thumbnailUrl: string) => {
        if (!token) return thumbnailUrl;
        const sep = thumbnailUrl.includes("?") ? "&" : "?";
        return `${thumbnailUrl}${sep}token=${token}`;
    };

    const formatDate = (iso: string) => {
        try {
            return new Date(iso).toLocaleDateString(undefined, {
                year: "numeric",
                month: "short",
                day: "numeric",
            });
        } catch {
            return iso;
        }
    };

    const formatSimilarity = (sim: number) => {
        if (sim >= 1.0) return "Exact";
        return `${(sim * 100).toFixed(1)}%`;
    };

    const similarityBadgeClass = (sim: number) => {
        if (sim >= 1.0) return "bg-purple-600 text-white";
        if (sim >= 0.98) return "bg-red-600 text-white";
        if (sim >= 0.95) return "bg-orange-500 text-white";
        return "bg-yellow-500 text-black";
    };

    return (
        <div>
            {/* Header */}
            <div className="mb-6 flex flex-col sm:flex-row sm:items-center gap-4">
                <div className="flex-1">
                    <h2 className="text-lg font-semibold text-gray-100">
                        {duplicatesStore.isLoading
                            ? "Scanning for duplicates…"
                            : `${duplicatesStore.groups.length} duplicate group${duplicatesStore.groups.length !== 1 ? "s" : ""} found`}
                    </h2>
                    {!isAdmin && (
                        <p className="text-xs text-gray-500 mt-1">
                            Admin access required to delete images.
                        </p>
                    )}
                </div>

                <div className="flex items-center gap-4">
                    {/* Threshold slider */}
                    <div className="flex items-center gap-3">
                        <label className="text-xs text-gray-400 whitespace-nowrap">
                            Min similarity: {(duplicatesStore.threshold * 100).toFixed(0)}%
                        </label>
                        <input
                            type="range"
                            min="80"
                            max="100"
                            step="1"
                            value={Math.round(duplicatesStore.threshold * 100)}
                            onChange={(e) =>
                                duplicatesStore.setThreshold(parseInt(e.target.value) / 100)
                            }
                            className="w-28 h-1.5 bg-gray-700 rounded-lg appearance-none cursor-pointer accent-blue-500"
                        />
                    </div>

                    {/* Refresh button */}
                    <button
                        onClick={() => duplicatesStore.fetchDuplicates()}
                        disabled={duplicatesStore.isLoading}
                        className="flex items-center gap-2 px-3 py-1.5 bg-blue-600 hover:bg-blue-700 disabled:bg-gray-700 disabled:text-gray-500 text-white text-sm rounded-md transition-colors"
                    >
                        <RefreshCw
                            size={14}
                            className={duplicatesStore.isLoading ? "animate-spin" : ""}
                        />
                        Refresh
                    </button>
                </div>
            </div>

            {/* Error */}
            {duplicatesStore.error && (
                <div className="mb-4 p-3 bg-red-900/30 border border-red-700 rounded-md text-red-400 text-sm">
                    {duplicatesStore.error}
                </div>
            )}

            {/* Loading spinner */}
            {duplicatesStore.isLoading && (
                <div className="flex items-center justify-center py-20">
                    <div className="w-8 h-8 border-2 border-blue-500 border-t-transparent rounded-full animate-spin" />
                </div>
            )}

            {/* Empty state */}
            {!duplicatesStore.isLoading && duplicatesStore.groups.length === 0 && (
                <div className="flex flex-col items-center justify-center py-20 text-gray-500">
                    <Copy size={48} className="mb-4 opacity-30" />
                    <p className="text-lg font-medium">No duplicates found</p>
                    <p className="text-sm mt-1">
                        Try lowering the similarity threshold to find more matches.
                    </p>
                </div>
            )}

            {/* Duplicate comparison lightbox */}
            {lightboxState !== null && duplicatesStore.groups[lightboxState.groupIdx] && (
                <DuplicatesLightbox
                    group={duplicatesStore.groups[lightboxState.groupIdx]}
                    initialIdx={lightboxState.imageIdx}
                    isAdmin={isAdmin}
                    token={token}
                    onClose={() => setLightboxState(null)}
                    onDelete={(hash) => duplicatesStore.deleteImage(hash)}
                    onPrev={lightboxState.groupIdx > 0
                        ? () => setLightboxState({ groupIdx: lightboxState.groupIdx - 1, imageIdx: 0 })
                        : undefined}
                    onNext={lightboxState.groupIdx < duplicatesStore.groups.length - 1
                        ? () => setLightboxState({ groupIdx: lightboxState.groupIdx + 1, imageIdx: 0 })
                        : undefined}
                />
            )}

            {/* Duplicate groups */}
            {!duplicatesStore.isLoading &&
                duplicatesStore.groups.map((group, groupIdx) => (
                    <div
                        key={groupIdx}
                        className="mb-6 border border-gray-700 rounded-lg overflow-hidden"
                    >
                        {/* Group header */}
                        <div className="flex items-center gap-3 px-4 py-2 bg-gray-800 border-b border-gray-700">
                            <span
                                className={`text-xs font-bold px-2 py-0.5 rounded ${similarityBadgeClass(group.similarity)}`}
                            >
                                {formatSimilarity(group.similarity)}
                            </span>
                            <span className="text-xs text-gray-400">
                                {group.images.length} copies
                            </span>
                        </div>

                        {/* Image cards — horizontal scroll on small screens */}
                        <div className="flex gap-3 p-3 overflow-x-auto">
                            {group.images.map((img, imgIdx) => (
                                <div
                                    key={`${img.hash}-${img.deviceid}-${imgIdx}`}
                                    className="flex-shrink-0 w-40 bg-gray-800 rounded-lg overflow-hidden border border-gray-700"
                                >
                                    {/* Thumbnail */}
                                    <div
                                        className="relative h-32 bg-gray-900 cursor-pointer"
                                        onClick={() => setLightboxState({ groupIdx, imageIdx: imgIdx })}
                                    >
                                        <img
                                            src={getThumbSrc(img.thumbnail_url)}
                                            alt={img.name}
                                            className="w-full h-full object-cover"
                                            loading="lazy"
                                        />
                                    </div>

                                    {/* Card info */}
                                    <div className="p-2">
                                        <p
                                            className="text-xs text-gray-200 truncate font-medium"
                                            title={img.name}
                                        >
                                            {img.name}
                                        </p>
                                        <p className="text-[10px] text-gray-500 mt-0.5">
                                            {formatDate(img.created_at)}
                                        </p>
                                        <p
                                            className="text-[10px] text-gray-600 truncate mt-0.5"
                                            title={img.deviceid}
                                        >
                                            {img.deviceid}
                                        </p>

                                        {isAdmin && (
                                            <button
                                                onClick={() => duplicatesStore.deleteImage(img.hash)}
                                                className="mt-2 w-full flex items-center justify-center gap-1.5 px-2 py-1 bg-red-900/40 hover:bg-red-700 text-red-400 hover:text-white text-xs rounded transition-colors"
                                                title="Delete this image"
                                            >
                                                <Trash2 size={11} />
                                                Delete
                                            </button>
                                        )}
                                    </div>
                                </div>
                            ))}
                        </div>
                    </div>
                ))}
        </div>
    );
});
