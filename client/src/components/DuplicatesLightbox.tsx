import { useCallback, useEffect, useRef, useState } from "react";
import { ZoomIn, ZoomOut, Maximize2, ArrowLeftRight, X, Trash2, ChevronLeft, ChevronRight, ChevronUp, ChevronDown, Star } from "lucide-react";
import type { DuplicateGroup, DuplicateImage } from "../stores/DuplicatesStore";

interface Props {
    group: DuplicateGroup;
    initialIdx: number;
    isAdmin: boolean;
    token: string | null;
    onClose: () => void;
    onDelete: (hash: string) => void;
    onPrev?: () => void;
    onNext?: () => void;
}

export function DuplicatesLightbox({ group, initialIdx, isAdmin, token, onClose, onDelete, onPrev, onNext }: Props) {
    const [leftIdx, setLeftIdx] = useState(initialIdx);
    const [rightIdx, setRightIdx] = useState(initialIdx === 0 ? 1 : 0);
    const [transform, setTransform] = useState({ scale: 1, tx: 0, ty: 0 });
    const [dragging, setDragging] = useState(false);
    const [dragStart, setDragStart] = useState({ x: 0, y: 0, tx: 0, ty: 0 });

    const leftRef = useRef<HTMLDivElement>(null);
    const rightRef = useRef<HTMLDivElement>(null);

    const getAuthUrl = (hash: string) => {
        const base = `/api/image/${hash}`;
        return token ? `${base}?token=${token}` : base;
    };

    // Close or clamp indices when images are deleted from the group
    useEffect(() => {
        if (group.images.length < 2) {
            onClose();
            return;
        }
        const maxIdx = group.images.length - 1;
        const newLeft = Math.min(leftIdx, maxIdx);
        let newRight = Math.min(rightIdx, maxIdx);
        if (newRight === newLeft) {
            newRight = newLeft === 0 ? 1 : 0;
        }
        if (newLeft !== leftIdx) setLeftIdx(newLeft);
        if (newRight !== rightIdx) setRightIdx(newRight);
    }, [group.images.length]); // eslint-disable-line react-hooks/exhaustive-deps

    // Reset zoom when displayed images change
    const leftHash = group.images[leftIdx]?.hash;
    const rightHash = group.images[rightIdx]?.hash;

    useEffect(() => {
        setTransform({ scale: 1, tx: 0, ty: 0 });
    }, [leftHash, rightHash]);

    // Non-passive wheel zoom (synced)
    useEffect(() => {
        const makeHandler = (el: HTMLDivElement) => (e: WheelEvent) => {
            e.preventDefault();
            const rect = el.getBoundingClientRect();
            const cx = e.clientX - rect.left;
            const cy = e.clientY - rect.top;
            const factor = e.deltaY < 0 ? 1.15 : 1 / 1.15;
            setTransform((prev) => {
                const s = Math.min(Math.max(prev.scale * factor, 0.5), 16);
                const r = s / prev.scale;
                return { scale: s, tx: cx - (cx - prev.tx) * r, ty: cy - (cy - prev.ty) * r };
            });
        };
        const lEl = leftRef.current!;
        const rEl = rightRef.current!;
        const lh = makeHandler(lEl);
        const rh = makeHandler(rEl);
        lEl.addEventListener("wheel", lh, { passive: false });
        rEl.addEventListener("wheel", rh, { passive: false });
        return () => {
            lEl.removeEventListener("wheel", lh);
            rEl.removeEventListener("wheel", rh);
        };
    }, []);

    // Cycle the left panel through images in the group; swap panels when hitting the right panel image
    const navigateImage = useCallback((dir: 1 | -1) => {
        const n = group.images.length;
        setLeftIdx((prevLeft) => {
            const next = ((prevLeft + dir) % n + n) % n;
            if (next === rightIdx) {
                setRightIdx(prevLeft);
            }
            return next;
        });
    }, [group.images.length, rightIdx]);

    // Keyboard navigation
    useEffect(() => {
        const handler = (e: KeyboardEvent) => {
            if (e.key === "Escape") onClose();
            if (e.key === "ArrowLeft")  navigateImage(-1);
            if (e.key === "ArrowRight") navigateImage(1);
            if (e.key === "ArrowUp")    onPrev?.();
            if (e.key === "ArrowDown")  onNext?.();
        };
        window.addEventListener("keydown", handler);
        return () => window.removeEventListener("keydown", handler);
    }, [onClose, onPrev, onNext, navigateImage]);

    const handleMouseDown = (e: React.MouseEvent) => {
        if (e.button !== 0) return;
        setDragging(true);
        setDragStart({ x: e.clientX, y: e.clientY, tx: transform.tx, ty: transform.ty });
    };

    const handleMouseMove = (e: React.MouseEvent) => {
        if (!dragging) return;
        setTransform((prev) => ({
            ...prev,
            tx: dragStart.tx + (e.clientX - dragStart.x),
            ty: dragStart.ty + (e.clientY - dragStart.y),
        }));
    };

    const handleMouseUp = () => setDragging(false);

    const handleThumbClick = (idx: number) => {
        if (idx === leftIdx) return;
        if (idx === rightIdx) {
            setLeftIdx(rightIdx);
            setRightIdx(leftIdx);
        } else {
            setLeftIdx(idx);
        }
    };

    const getThumbSrc = (thumbnailUrl: string) => {
        if (!token || thumbnailUrl.includes("token=")) return thumbnailUrl;
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

    const formatFileSize = (b: number) =>
        b >= 1_000_000 ? `${(b / 1_000_000).toFixed(1)} MB` : `${Math.round(b / 1000)} KB`;

    const sharpnessLabel = (s: number) => s > 500 ? "Sharp" : s > 100 ? "OK" : "Blurry";

    const qualityScore = (img: DuplicateImage): number | null => {
        if (img.aesthetic_score == null || img.sharpness_score == null) return null;
        return img.aesthetic_score / 10 * 0.6 + Math.min(img.sharpness_score / 1000, 1) * 0.4;
    };

    const similarityBadgeClass = (sim: number) => {
        if (sim >= 1.0) return "bg-purple-600 text-white";
        if (sim >= 0.98) return "bg-red-600 text-white";
        if (sim >= 0.95) return "bg-orange-500 text-white";
        return "bg-yellow-500 text-black";
    };

    if (group.images.length < 2) return null;
    const leftImg = group.images[leftIdx];
    const rightImg = group.images[rightIdx];
    if (!leftImg || !rightImg) return null;

    const imgTransformStyle = {
        transform: `translate(${transform.tx}px, ${transform.ty}px) scale(${transform.scale})`,
        transformOrigin: "0 0" as const,
    };

    return (
        <div
            className="fixed inset-0 z-50 flex flex-col bg-black"
            onMouseMove={handleMouseMove}
            onMouseUp={handleMouseUp}
            onMouseLeave={handleMouseUp}
        >
            {/* Top bar */}
            <div className="flex-shrink-0 flex items-center justify-between px-4 py-2 bg-gray-900 border-b border-gray-700">
                <div className="flex items-center gap-3">
                    <span className={`text-xs font-bold px-2 py-0.5 rounded ${similarityBadgeClass(group.similarity)}`}>
                        {formatSimilarity(group.similarity)}
                    </span>
                    <span className="text-xs text-gray-400">{group.images.length} images</span>
                    {/* Group navigation */}
                    <div className="flex items-center gap-0.5">
                        <button
                            onClick={onPrev}
                            disabled={!onPrev}
                            className="p-1 text-gray-400 hover:text-white hover:bg-gray-700 disabled:opacity-30 disabled:cursor-default rounded"
                            title="Previous group (↑)"
                        >
                            <ChevronUp size={15} />
                        </button>
                        <button
                            onClick={onNext}
                            disabled={!onNext}
                            className="p-1 text-gray-400 hover:text-white hover:bg-gray-700 disabled:opacity-30 disabled:cursor-default rounded"
                            title="Next group (↓)"
                        >
                            <ChevronDown size={15} />
                        </button>
                    </div>
                </div>
                <div className="flex items-center gap-1">
                    <button
                        onClick={() => setTransform((p) => ({ ...p, scale: Math.min(p.scale * 1.25, 16) }))}
                        className="p-1.5 text-gray-300 hover:text-white hover:bg-gray-700 rounded"
                        title="Zoom in"
                    >
                        <ZoomIn size={16} />
                    </button>
                    <button
                        onClick={() => setTransform((p) => ({ ...p, scale: Math.max(p.scale / 1.25, 0.5) }))}
                        className="p-1.5 text-gray-300 hover:text-white hover:bg-gray-700 rounded"
                        title="Zoom out"
                    >
                        <ZoomOut size={16} />
                    </button>
                    <button
                        onClick={() => setTransform({ scale: 1, tx: 0, ty: 0 })}
                        className="p-1.5 text-gray-300 hover:text-white hover:bg-gray-700 rounded"
                        title="Reset zoom (1:1)"
                    >
                        <Maximize2 size={16} />
                    </button>
                    <button
                        onClick={() => { setLeftIdx(rightIdx); setRightIdx(leftIdx); }}
                        className="p-1.5 text-gray-300 hover:text-white hover:bg-gray-700 rounded"
                        title="Swap panels"
                    >
                        <ArrowLeftRight size={16} />
                    </button>
                    {isAdmin && (() => {
                        // Find the image with highest quality score to keep; delete the rest
                        const best = group.images.reduce<DuplicateImage | null>((b, img) => {
                            const qs = qualityScore(img);
                            const bqs = b ? qualityScore(b) : null;
                            if (qs == null) return b;
                            if (bqs == null || qs > bqs) return img;
                            return b;
                        }, null);
                        if (!best) return null;
                        const toDelete = group.images.filter((img) => img.hash !== best.hash);
                        if (toDelete.length === 0) return null;
                        return (
                            <button
                                onClick={() => toDelete.forEach((img) => onDelete(img.hash))}
                                className="flex items-center gap-1.5 px-2 py-1 bg-green-800/50 hover:bg-green-700 text-green-300 hover:text-white text-xs rounded transition-colors"
                                title={`Keep best quality (${best.name}) and delete ${toDelete.length} other${toDelete.length !== 1 ? "s" : ""}`}
                            >
                                <Star size={12} />
                                Keep best
                            </button>
                        );
                    })()}
                    <div className="w-px h-5 bg-gray-700 mx-1" />
                    <button
                        onClick={onClose}
                        className="p-1.5 text-gray-300 hover:text-white hover:bg-gray-700 rounded"
                        title="Close"
                    >
                        <X size={16} />
                    </button>
                </div>
            </div>

            {/* Main comparison area */}
            <div
                className={`relative flex flex-1 min-h-0 select-none ${dragging ? "cursor-grabbing" : "cursor-grab"}`}
                onMouseDown={handleMouseDown}
            >
                {/* Prev image arrow */}
                <button
                    onClick={(e) => { e.stopPropagation(); navigateImage(-1); }}
                    onMouseDown={(e) => e.stopPropagation()}
                    className="absolute left-2 top-1/2 -translate-y-1/2 z-20 p-2 bg-black/60 hover:bg-black/90 text-white rounded-full transition-colors"
                    title="Previous image (←)"
                >
                    <ChevronLeft size={24} />
                </button>
                {/* Next image arrow */}
                <button
                    onClick={(e) => { e.stopPropagation(); navigateImage(1); }}
                    onMouseDown={(e) => e.stopPropagation()}
                    className="absolute right-2 top-1/2 -translate-y-1/2 z-20 p-2 bg-black/60 hover:bg-black/90 text-white rounded-full transition-colors"
                    title="Next image (→)"
                >
                    <ChevronRight size={24} />
                </button>
                {/* Left panel */}
                <div ref={leftRef} className="flex-1 flex flex-col overflow-hidden bg-black min-w-0">
                    <div className="relative flex-1 min-h-0">
                        <div className="absolute top-2 left-2 z-10 bg-blue-600 text-white text-xs font-bold px-1.5 py-0.5 rounded pointer-events-none">
                            L
                        </div>
                        <img
                            src={getAuthUrl(leftImg.hash)}
                            alt={leftImg.name}
                            className="absolute inset-0 w-full h-full object-contain pointer-events-none"
                            style={imgTransformStyle}
                        />
                    </div>
                    <div
                        className="flex-shrink-0 px-3 py-2 bg-gray-900/90 border-t border-gray-700 flex items-center gap-2"
                        onMouseDown={(e) => e.stopPropagation()}
                    >
                        <div className="flex-1 min-w-0">
                            <p className="text-xs text-gray-200 truncate font-medium" title={leftImg.name}>
                                {leftImg.name}
                            </p>
                            <p className="text-[10px] text-gray-400">{formatDate(leftImg.created_at)}</p>
                            {(leftImg.width || leftImg.file_size_bytes) && (
                                <div className="flex gap-2 mt-1 flex-wrap">
                                    {leftImg.width && <span className="text-[10px] text-gray-400">{leftImg.width}×{leftImg.height}</span>}
                                    {leftImg.file_size_bytes && <span className="text-[10px] text-gray-400">{formatFileSize(leftImg.file_size_bytes)}</span>}
                                    {leftImg.sharpness_score != null && <span className="text-[10px] text-gray-400">{sharpnessLabel(leftImg.sharpness_score)}</span>}
                                    {leftImg.aesthetic_score != null && (
                                        <span className="text-[10px] text-yellow-400">★ {leftImg.aesthetic_score.toFixed(1)}</span>
                                    )}
                                </div>
                            )}
                            {(() => {
                                const lq = qualityScore(leftImg), rq = qualityScore(rightImg);
                                return lq != null && rq != null && lq > rq
                                    ? <span className="text-[10px] text-green-400 font-semibold">✓ Better quality</span>
                                    : null;
                            })()}
                        </div>
                        {isAdmin && (
                            <button
                                onClick={() => onDelete(leftImg.hash)}
                                className="flex-shrink-0 flex items-center gap-1 px-2 py-1 bg-red-900/40 hover:bg-red-700 text-red-400 hover:text-white text-xs rounded transition-colors"
                            >
                                <Trash2 size={11} />
                                Delete
                            </button>
                        )}
                    </div>
                </div>

                {/* Divider */}
                <div className="w-px bg-gray-700 flex-shrink-0" />

                {/* Right panel */}
                <div ref={rightRef} className="flex-1 flex flex-col overflow-hidden bg-black min-w-0">
                    <div className="relative flex-1 min-h-0">
                        <div className="absolute top-2 left-2 z-10 bg-orange-500 text-white text-xs font-bold px-1.5 py-0.5 rounded pointer-events-none">
                            R
                        </div>
                        <img
                            src={getAuthUrl(rightImg.hash)}
                            alt={rightImg.name}
                            className="absolute inset-0 w-full h-full object-contain pointer-events-none"
                            style={imgTransformStyle}
                        />
                    </div>
                    <div
                        className="flex-shrink-0 px-3 py-2 bg-gray-900/90 border-t border-gray-700 flex items-center gap-2"
                        onMouseDown={(e) => e.stopPropagation()}
                    >
                        <div className="flex-1 min-w-0">
                            <p className="text-xs text-gray-200 truncate font-medium" title={rightImg.name}>
                                {rightImg.name}
                            </p>
                            <p className="text-[10px] text-gray-400">{formatDate(rightImg.created_at)}</p>
                            {(rightImg.width || rightImg.file_size_bytes) && (
                                <div className="flex gap-2 mt-1 flex-wrap">
                                    {rightImg.width && <span className="text-[10px] text-gray-400">{rightImg.width}×{rightImg.height}</span>}
                                    {rightImg.file_size_bytes && <span className="text-[10px] text-gray-400">{formatFileSize(rightImg.file_size_bytes)}</span>}
                                    {rightImg.sharpness_score != null && <span className="text-[10px] text-gray-400">{sharpnessLabel(rightImg.sharpness_score)}</span>}
                                    {rightImg.aesthetic_score != null && (
                                        <span className="text-[10px] text-yellow-400">★ {rightImg.aesthetic_score.toFixed(1)}</span>
                                    )}
                                </div>
                            )}
                            {(() => {
                                const lq = qualityScore(leftImg), rq = qualityScore(rightImg);
                                return rq != null && lq != null && rq > lq
                                    ? <span className="text-[10px] text-green-400 font-semibold">✓ Better quality</span>
                                    : null;
                            })()}
                        </div>
                        {isAdmin && (
                            <button
                                onClick={() => onDelete(rightImg.hash)}
                                className="flex-shrink-0 flex items-center gap-1 px-2 py-1 bg-red-900/40 hover:bg-red-700 text-red-400 hover:text-white text-xs rounded transition-colors"
                            >
                                <Trash2 size={11} />
                                Delete
                            </button>
                        )}
                    </div>
                </div>
            </div>

            {/* Thumbnail strip */}
            <div
                className="flex-shrink-0 bg-gray-900 border-t border-gray-700 overflow-x-auto"
                onMouseDown={(e) => e.stopPropagation()}
            >
                <div className="flex gap-2 p-2">
                    {group.images.map((img, idx) => {
                        const isLeft = idx === leftIdx;
                        const isRight = idx === rightIdx;
                        return (
                            <div
                                key={`${img.hash}-${idx}`}
                                onClick={() => handleThumbClick(idx)}
                                className={`relative flex-shrink-0 w-16 h-16 rounded overflow-hidden border-2 cursor-pointer transition-colors ${
                                    isLeft
                                        ? "border-blue-500"
                                        : isRight
                                          ? "border-orange-500"
                                          : "border-gray-600 hover:border-gray-400"
                                }`}
                                title={img.name}
                            >
                                <img
                                    src={getThumbSrc(img.thumbnail_url)}
                                    alt={img.name}
                                    className="w-full h-full object-cover"
                                />
                                {isLeft && (
                                    <div className="absolute bottom-0 left-0 bg-blue-600 text-white text-[8px] font-bold px-1 leading-tight">
                                        L
                                    </div>
                                )}
                                {isRight && (
                                    <div className="absolute bottom-0 right-0 bg-orange-500 text-white text-[8px] font-bold px-1 leading-tight">
                                        R
                                    </div>
                                )}
                            </div>
                        );
                    })}
                </div>
            </div>
        </div>
    );
}
