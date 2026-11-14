import { observer } from "mobx-react-lite";
import { useStore } from "../stores/RootStore";
import { useEffect, useRef, useState } from "react";
import { RefreshCw, X, ChevronLeft, ChevronRight, Trash2, Loader, AlertTriangle, RotateCcw, RotateCw } from "lucide-react";

// "Orientation check" tab — lists images that carry NO EXIF metadata (so they have no
// EXIF orientation and rely on AI orientation detection). Lets you review them manually,
// filter to only images detected with non-upright orientation, rotate, and delete if needed.
function formatOrientationBadge(orientation?: number | null): string | null {
    if (!orientation || orientation <= 1) return null;
    switch (orientation) {
        case 2:
            return "Flip H";
        case 3:
            return "180°";
        case 4:
            return "Flip V";
        case 5:
            return "Transpose";
        case 6:
            return "90° CW";
        case 7:
            return "Transverse";
        case 8:
            return "90° CCW";
        default:
            return `Orient ${orientation}`;
    }
}

export const OrientationCheck = observer(() => {
    const { mediaStore, authStore } = useStore();
    const isAdmin = authStore.user?.role === "admin";

    const [selected, setSelected] = useState<number | null>(null);
    const [fullUrl, setFullUrl] = useState<string | null>(null);
    const observerTarget = useRef<HTMLDivElement>(null);

    const images = mediaStore.noExifImages;

    useEffect(() => {
        mediaStore.fetchNoExifImages(1, 50, false);
    }, [mediaStore]);

    useEffect(() => {
        const el = observerTarget.current;
        if (!el) return;
        const io = new IntersectionObserver((entries) => {
            if (entries[0].isIntersecting) mediaStore.loadMoreNoExif();
        }, { rootMargin: "600px" });
        io.observe(el);
        return () => io.disconnect();
    }, [mediaStore]);

    const openFull = async (index: number) => {
        setSelected(index);
        const item = images[index];
        if (!item) return;
        const url = item.media_type === "video" ? `/api/video/${item.hash}` : `/api/image/${item.hash}`;
        setFullUrl(url);
    };

    const closeFull = () => {
        setFullUrl(null);
        setSelected(null);
    };

    const nav = (dir: 1 | -1) => {
        if (selected === null || images.length === 0) return;
        const next = (selected + dir + images.length) % images.length;
        openFull(next);
    };

    const handleKeyRef = useRef<(e: KeyboardEvent) => void>(() => {});
    handleKeyRef.current = (e: KeyboardEvent) => {
        if (selected === null) return;
        if (e.key === 'Escape') closeFull();
        if (e.key === 'ArrowRight') nav(1);
        if (e.key === 'ArrowLeft') nav(-1);
    };

    useEffect(() => {
        const onKeyDown = (e: KeyboardEvent) => handleKeyRef.current(e);
        window.addEventListener('keydown', onKeyDown);
        return () => window.removeEventListener('keydown', onKeyDown);
    }, []);

    const handleDelete = async () => {
        if (selected === null || !images[selected]) return;
        const item = images[selected];
        if (!window.confirm(`Delete "${item.name}"?`)) return;
        closeFull();
        await mediaStore.deleteMedia(item.hash);
        // Refetch first, then clamp the viewer index to the new list bounds so
        // the next rendered item is a real neighbour (no arbitrary jump).
        await mediaStore.fetchNoExifImages(1, 50, false);
        const remaining = mediaStore.noExifImages;
        if (remaining.length === 0) {
            setSelected(null);
            return;
        }
        const nextIndex = Math.min(selected, remaining.length - 1);
        setSelected(nextIndex);
        const nextItem = remaining[nextIndex];
        setFullUrl(nextItem.media_type === "video" ? `/api/video/${nextItem.hash}` : `/api/image/${nextItem.hash}`);
    };

    return (
        <div className="animate-in fade-in slide-in-from-bottom-2 duration-500">
            <div className="flex flex-wrap items-center justify-between gap-3 mb-6">
                <div>
                    <h1 className="text-xl font-bold text-gray-100">Orientation Check</h1>
                    <p className="text-sm text-gray-400 mt-1">
                        {mediaStore.noExifWrongOrientationOnly
                            ? `${mediaStore.totalNoExif.toLocaleString()} photos with no EXIF metadata detected with non-upright orientation — review manually.`
                            : `${mediaStore.totalNoExif.toLocaleString()} photos with no EXIF metadata (AI orientation fallback) — review manually.`}
                    </p>
                </div>
                <div className="flex items-center gap-2">
                    <button
                        type="button"
                        aria-pressed={mediaStore.noExifWrongOrientationOnly}
                        onClick={() => {
                            closeFull();
                            void mediaStore.setNoExifWrongOrientationOnly(!mediaStore.noExifWrongOrientationOnly);
                        }}
                        className={`min-h-9 px-3 py-2 rounded-lg text-sm font-medium flex items-center gap-2 border transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-blue-400 ${
                            mediaStore.noExifWrongOrientationOnly
                                ? "bg-blue-600/20 border-blue-500 text-blue-200 hover:bg-blue-600/30"
                                : "bg-gray-800 border-gray-700 text-gray-300 hover:bg-gray-700"
                        }`}
                    >
                        <span
                            aria-hidden="true"
                            className={`w-2.5 h-2.5 rounded-full ${
                                mediaStore.noExifWrongOrientationOnly ? "bg-blue-400" : "bg-gray-500"
                            }`}
                        />
                        Wrong orientation only
                    </button>
                    <button
                        type="button"
                        onClick={() => mediaStore.fetchNoExifImages(1, 50, false)}
                        className="min-h-9 px-3 py-2 rounded-lg bg-gray-800 hover:bg-gray-700 text-gray-300 text-sm flex items-center gap-2 transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-blue-400"
                    >
                        <RefreshCw size={16} aria-hidden="true" className="text-blue-400" />
                        Refresh
                    </button>
                </div>
            </div>

            {!mediaStore.isLoadingNoExif && images.length === 0 && mediaStore.totalNoExif === 0 ? (
                <div className="flex flex-col items-center justify-center py-24 text-gray-500 gap-3">
                    <AlertTriangle aria-hidden="true" className="w-10 h-10 text-gray-600" />
                    <p className="text-sm text-center">
                        {mediaStore.noExifWrongOrientationOnly
                            ? "No photos without EXIF metadata were detected with wrong orientation."
                            : "No photos without EXIF metadata found."}
                    </p>
                    {mediaStore.noExifWrongOrientationOnly && (
                        <button
                            type="button"
                            onClick={() => void mediaStore.setNoExifWrongOrientationOnly(false)}
                            className="min-h-9 px-3 py-2 rounded-lg bg-gray-800 hover:bg-gray-700 text-gray-200 text-sm transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-blue-400"
                        >
                            Show all photos without EXIF
                        </button>
                    )}
                </div>
            ) : (
                <div className="grid grid-cols-3 md:grid-cols-4 lg:grid-cols-6 gap-2">
                    {images.map((item, idx) => {
                        const badge = formatOrientationBadge(item.orientation);
                        return (
                        <div
                            key={`${item.hash}_${idx}`}
                            className="group relative aspect-square bg-gray-800 rounded-xl overflow-hidden cursor-pointer hover:ring-2 ring-blue-500 transition-all shadow-lg"
                            tabIndex={0}
                            role="button"
                            aria-label={`Open ${item.name}`}
                            onClick={() => openFull(idx)}
                            onKeyDown={(e) => { if (e.key === 'Enter') openFull(idx); }}
                        >
                            <img src={item.thumbnailUrl} alt={item.name} className="object-cover w-full h-full group-hover:scale-110 transition-transform duration-500" loading="lazy" />
                            {badge && (
                                <span className="absolute bottom-2 left-2 px-1.5 py-0.5 rounded bg-black/75 text-amber-300 text-[11px] font-medium pointer-events-none">
                                    {badge}
                                </span>
                            )}
                            {isAdmin && (
                                <button
                                    type="button"
                                    onClick={(e) => {
                                        e.stopPropagation();
                                        if (window.confirm(`Delete "${item.name}"?`)) {
                                            mediaStore.deleteMedia(item.hash).then(() => mediaStore.fetchNoExifImages(1, 50, false));
                                        }
                                    }}
                                    className="absolute top-2 right-2 p-1.5 rounded-full bg-black/50 opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 [@media(hover:none)]:opacity-100 hover:bg-red-900/80 transition-opacity"
                                    title="Delete"
                                    aria-label={`Delete ${item.name}`}
                                >
                                    <Trash2 size={14} aria-hidden="true" className="text-red-400" />
                                </button>
                            )}
                        </div>
                        );
                    })}
                </div>
            )}

            <div ref={observerTarget} className="h-20 flex items-center justify-center">
                {mediaStore.isLoadingNoExif && <Loader className="w-6 h-6 text-blue-500 animate-spin" />}
                {!mediaStore.noExifHasMore && images.length > 0 && (
                    <span className="text-xs text-gray-600">End of list</span>
                )}
            </div>

            {selected !== null && images[selected] && (
                <div
                    role="dialog"
                    aria-modal="true"
                    aria-label="Image orientation viewer"
                    className="fixed inset-0 z-50 bg-black/90 flex flex-col overscroll-contain"
                >
                    <div className="flex items-center justify-between p-3 bg-black/80">
                        <span className="text-sm text-gray-300 truncate flex-1">
                            {selected + 1} / {images.length} — {images[selected]?.name}
                            {formatOrientationBadge(images[selected]?.orientation) && (
                                <span className="ml-2 px-1.5 py-0.5 rounded bg-amber-500/20 text-amber-300 text-xs">
                                    {formatOrientationBadge(images[selected]?.orientation)}
                                </span>
                            )}
                        </span>
                        <button
                            onClick={async () => {
                                const item = images[selected];
                                if (!item) return;
                                await mediaStore.updateImageOrientation(item.hash, "ccw");
                                setFullUrl(`/api/image/${item.hash}?v=${Date.now()}`);
                            }}
                            className="p-2 rounded-md hover:bg-gray-700 text-gray-300 mr-1"
                            title="Rotate 90° CCW"
                            aria-label="Rotate 90° CCW"
                        >
                            <RotateCcw size={20} />
                        </button>
                        <button
                            onClick={async () => {
                                const item = images[selected];
                                if (!item) return;
                                await mediaStore.updateImageOrientation(item.hash, "cw");
                                setFullUrl(`/api/image/${item.hash}?v=${Date.now()}`);
                            }}
                            className="p-2 rounded-md hover:bg-gray-700 text-gray-300 mr-1"
                            title="Rotate 90° CW"
                            aria-label="Rotate 90° CW"
                        >
                            <RotateCw size={20} />
                        </button>
                        {isAdmin && (
                            <button onClick={handleDelete} className="p-2 rounded-md hover:bg-red-900/60 text-red-400 mr-1" title="Delete" aria-label="Delete image">
                                <Trash2 size={20} />
                            </button>
                        )}
                        <button onClick={closeFull} className="p-2 rounded-md hover:bg-gray-700 text-gray-300" title="Close (Esc)" aria-label="Close viewer">
                            <X size={24} />
                        </button>
                    </div>
                    <div className="flex items-center justify-center flex-1 min-h-0 relative">
                        <button onClick={() => nav(-1)} className="absolute left-3 p-2 rounded-full bg-black/50 hover:bg-black/80 text-white" title="Previous" aria-label="Previous image">
                            <ChevronLeft size={28} />
                        </button>
                        <div className="max-h-full max-w-full flex items-center justify-center p-4">
                            {fullUrl ? (
                                <img src={fullUrl} alt={images[selected]?.name} className="max-h-full max-w-full object-contain rounded shadow-2xl" />
                            ) : (
                                <Loader className="w-8 h-8 text-blue-500 animate-spin" />
                            )}
                        </div>
                        <button type="button" onClick={() => nav(1)} className="absolute right-3 p-2 rounded-full bg-black/50 hover:bg-black/80 text-white" title="Next" aria-label="Next image">
                            <ChevronRight size={28} aria-hidden="true" />
                        </button>
                    </div>
                </div>
            )}
        </div>
    );
});
