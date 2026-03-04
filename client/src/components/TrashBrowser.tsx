import { useEffect } from "react";
import { observer } from "mobx-react-lite";
import { useStore } from "../stores/RootStore";
import { Trash2, RotateCcw } from "lucide-react";

export const TrashBrowser = observer(() => {
    const { trashStore } = useStore();

    useEffect(() => {
        trashStore.fetchTrash();
    }, [trashStore]);

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

    if (trashStore.isLoading) {
        return (
            <div className="flex items-center justify-center py-20">
                <div className="w-8 h-8 border-2 border-blue-500 border-t-transparent rounded-full animate-spin" />
            </div>
        );
    }

    if (trashStore.error) {
        return (
            <div className="mb-4 p-3 bg-red-900/30 border border-red-700 rounded-md text-red-400 text-sm">
                {trashStore.error}
            </div>
        );
    }

    if (trashStore.items.length === 0) {
        return (
            <div className="flex flex-col items-center justify-center py-20 text-gray-500">
                <Trash2 size={48} className="mb-4 opacity-30" />
                <p className="text-lg font-medium">Trash is empty</p>
            </div>
        );
    }

    return (
        <div>
            <div className="mb-4">
                <h2 className="text-lg font-semibold text-gray-100">
                    {trashStore.items.length} deleted item{trashStore.items.length !== 1 ? "s" : ""}
                </h2>
            </div>

            <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6 gap-2">
                {trashStore.items.map((item) => (
                    <div
                        key={`${item.hash}-${item.media_type}`}
                        className="relative group aspect-square bg-gray-800 rounded overflow-hidden"
                    >
                        <img
                            src={trashStore.getThumbnailUrl(item)}
                            alt={item.name}
                            className="w-full h-full object-cover"
                            loading="lazy"
                        />
                        {/* Hover overlay */}
                        <div className="absolute inset-0 bg-black/70 opacity-0 group-hover:opacity-100 transition-opacity flex flex-col justify-between p-2">
                            <div>
                                <p className="text-xs text-gray-200 truncate font-medium" title={item.name}>
                                    {item.name}
                                </p>
                                <p className="text-[10px] text-gray-400 mt-0.5">
                                    Deleted {formatDate(item.deleted_at)}
                                </p>
                            </div>
                            <button
                                onClick={() => trashStore.restoreItem(item.hash, item.media_type)}
                                className="flex items-center justify-center gap-1.5 px-2 py-1.5 bg-blue-600 hover:bg-blue-500 text-white text-xs rounded transition-colors"
                                title="Restore"
                            >
                                <RotateCcw size={12} />
                                Restore
                            </button>
                        </div>
                    </div>
                ))}
            </div>
        </div>
    );
});
