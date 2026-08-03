import { observer } from "mobx-react-lite";
import { useStore } from "../stores/RootStore";
import { useEffect, useRef, useState } from "react";
import { RefreshCw, X, ChevronLeft, ChevronRight, Trash2, Loader, AlertTriangle } from "lucide-react";

// "Orientation check" tab — lists images that carry NO EXIF metadata (so they have no
// EXIF orientation and rely on AI orientation detection). Lets you review them manually,
// rotate check, and delete if needed.
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
        if (fullUrl?.startsWith('blob:')) URL.revokeObjectURL(fullUrl);
        const url = mediaStore.getAuthenticatedUrl(
            item.media_type === "video" ? `/api/video/${item.hash}` : `/api/image/${item.hash}`
        );
        setFullUrl(url);
    };

    const closeFull = () => {
        if (fullUrl?.startsWith('blob:')) URL.revokeObjectURL(fullUrl);
        setFullUrl(null);
        setSelected(null);
    };

    const nav = (dir: 1 | -1) => {
        if (selected === null || images.length === 0) return;
        const next = (selected + dir + images.length) % images.length;
        openFull(next);
    };

    const handleKey = (e: KeyboardEvent) => {
        if (selected === null) return;
        if (e.key === 'Escape') closeFull();
        if (e.key === 'ArrowRight') nav(1);
        if (e.key === 'ArrowLeft') nav(-1);
    };

    useEffect(() => {
        window.addEventListener('keydown', handleKey);
        return () => window.removeEventListener('keydown', handleKey);
    }, [selected, images.length]);

    const handleDelete = async () => {
        if (selected === null || !images[selected]) return;
        const item = images[selected];
        if (!window.confirm(`Delete "${item.name}"?`)) return;
        closeFull();
        await mediaStore.deleteMedia(item.hash);
        const remains = images.length > 1 ? selected % (images.length - 1) : null;
        setSelected(remains);
        if (remains !== null) openFull(remains);
        mediaStore.fetchNoExifImages(1, 50, false);
    };

    return (
        <div className="animate-in fade-in slide-in-from-bottom-2 duration-500">
            <div className="flex items-center justify-between mb-6">
                <div>
                    <h1 className="text-xl font-bold text-gray-100">Orientation Check</h1>
                    <p className="text-sm text-gray-400 mt-1">
                        {mediaStore.totalNoExif.toLocaleString()} photos with no EXIF metadata (AI orientation fallback) — review manually.
                    </p>
                </div>
                <button
                    onClick={() => mediaStore.fetchNoExifImages(1, 50, false)}
                    className="px-3 py-2 rounded-lg bg-gray-800 hover:bg-gray-700 text-gray-300 text-sm flex items-center gap-2 transition-colors"
                >
                    <RefreshCw size={16} className="text-blue-400" />
                    Refresh
                </button>
            </div>

            {images.length === 0 && mediaStore.totalNoExif === 0 ? (
                <div className="flex flex-col items-center justify-center py-24 text-gray-500">
                    <AlertTriangle className="w-10 h-10 mb-3 text-gray-600" />
                    <p className="text-sm">No photos without EXIF metadata found.</p>
                </div>
            ) : (
                <div className="grid grid-cols-3 md:grid-cols-4 lg:grid-cols-6 gap-2">
                    {images.map((item, idx) => (
                        <div
                            key={`${item.hash}_${idx}`}
                            className="group relative aspect-square bg-gray-800 rounded-xl overflow-hidden cursor-pointer hover:ring-2 ring-blue-500 transition-all shadow-lg"
                            tabIndex={0}
                            role="button"
                            aria-label={`Open ${item.name}`}
                            onClick={() => openFull(idx)}
                            onKeyDown={(e) => { if (e.key === 'Enter') openFull(idx); }}
                        >
                            {item.thumbnailUrl ? (
                                <img src={item.thumbnailUrl} alt={item.name} className="object-cover w-full h-full group-hover:scale-110 transition-transform duration-500" loading="lazy" />
                            ) : (
                                <div className="flex items-center justify-center h-full text-gray-600 italic text-xs">Loading...</div>
                            )}
                            {isAdmin && (
                                <button
                                    onClick={(e) => {
                                        e.stopPropagation();
                                        if (window.confirm(`Delete "${item.name}"?`)) {
                                            mediaStore.deleteMedia(item.hash).then(() => mediaStore.fetchNoExifImages(1, 50, false));
                                        }
                                    }}
                                    className="absolute top-2 right-2 p-1.5 rounded-full bg-black/50 opacity-0 group-hover:opacity-100 hover:bg-red-900/80 transition-opacity"
                                    title="Delete"
                                >
                                    <Trash2 size={14} className="text-red-400" />
                                </button>
                            )}
                        </div>
                    ))}
                </div>
            )}

            <div ref={observerTarget} className="h-20 flex items-center justify-center">
                {mediaStore.isLoadingNoExif && <Loader className="w-6 h-6 text-blue-500 animate-spin" />}
                {!mediaStore.noExifHasMore && images.length > 0 && (
                    <span className="text-xs text-gray-600">End of list</span>
                )}
            </div>

            {selected !== null && images[selected] && (
                <div className="fixed inset-0 z-50 bg-black/90 flex flex-col" onClick={() => {}}>
                    <div className="flex items-center justify-between p-3 bg-black/80">
                        <span className="text-sm text-gray-300 truncate flex-1">
                            {selected + 1} / {images.length} — {images[selected]?.name}
                        </span>
                        {isAdmin && (
                            <button onClick={handleDelete} className="p-2 rounded-md hover:bg-red-900/60 text-red-400 mr-1" title="Delete">
                                <Trash2 size={20} />
                            </button>
                        )}
                        <button onClick={closeFull} className="p-2 rounded-md hover:bg-gray-700 text-gray-300" title="Close (Esc)">
                            <X size={24} />
                        </button>
                    </div>
                    <div className="flex items-center justify-center flex-1 min-h-0 relative">
                        <button onClick={() => nav(-1)} className="absolute left-3 p-2 rounded-full bg-black/50 hover:bg-black/80 text-white" title="Previous">
                            <ChevronLeft size={28} />
                        </button>
                        <div className="max-h-full max-w-full flex items-center justify-center p-4">
                            {fullUrl ? (
                                <img src={fullUrl} alt={images[selected]?.name} className="max-h-full max-w-full object-contain rounded shadow-2xl" />
                            ) : (
                                <Loader className="w-8 h-8 text-blue-500 animate-spin" />
                            )}
                        </div>
                        <button onClick={() => nav(1)} className="absolute right-3 p-2 rounded-full bg-black/50 hover:bg-black/80 text-white" title="Next">
                            <ChevronRight size={28} />
                        </button>
                    </div>
                </div>
            )}
        </div>
    );
});
