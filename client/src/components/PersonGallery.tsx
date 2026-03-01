import { useEffect, useState, useCallback } from "react";
import { observer } from "mobx-react-lite";
import { useStore } from "../stores/RootStore";
import { User, Edit2, Check, X } from "lucide-react";
import { useNavigate } from "react-router-dom";

export const PersonGallery = observer(() => {
    const { personStore } = useStore();
    const [editingPersonId, setEditingPersonId] = useState<number | null>(null);
    const [editName, setEditName] = useState("");
    const navigate = useNavigate();

    const handleScroll = useCallback(() => {
        if (
            window.innerHeight + window.scrollY >= document.body.offsetHeight - 500 &&
            !personStore.isLoading &&
            personStore.hasMore
        ) {
            personStore.fetchPersons();
        }
    }, [personStore]);

    useEffect(() => {
        personStore.fetchPersons(true);
        window.addEventListener('scroll', handleScroll);

        return () => {
            window.removeEventListener('scroll', handleScroll);
            // Cleanup thumbnails on unmount
            personStore.cleanup();
        };
    }, [personStore, handleScroll]);

    const handleEditName = (personId: number, currentName: string | null) => {
        setEditingPersonId(personId);
        setEditName(currentName || "");
    };

    const handleSaveName = async (personId: number) => {
        if (editName.trim()) {
            await personStore.updatePersonName(personId, editName.trim());
        }
        setEditingPersonId(null);
        setEditName("");
    };

    const handleCancelEdit = () => {
        setEditingPersonId(null);
        setEditName("");
    };

    if (personStore.isLoading && personStore.persons.length === 0) {
        return (
            <div className="flex items-center justify-center h-64">
                <div className="text-gray-400">Loading persons...</div>
            </div>
        );
    }

    if (personStore.persons.length === 0) {
        return (
            <div className="flex flex-col items-center justify-center h-64 text-gray-400">
                <User size={48} className="mb-4" />
                <p>No persons detected yet</p>
                <p className="text-sm mt-2">Upload images with faces to get started</p>
            </div>
        );
    }

    return (
        <div>
            <div className="mb-6">
                <h2 className="text-2xl font-bold text-gray-200">
                    People ({personStore.persons.length})
                </h2>
                <p className="text-sm text-gray-400 mt-1">
                    Click on a person to see all their photos
                </p>
            </div>

            <div className="grid grid-cols-2 md:grid-cols-4 lg:grid-cols-6 gap-2">
                {personStore.persons.map((person) => (
                    <div
                        key={person.id}
                        className="group relative bg-gray-800 rounded-lg overflow-hidden cursor-pointer hover:ring-2 hover:ring-blue-500 transition-all"
                        onClick={() => navigate(`/people/${person.id}`)}
                    >
                        {/* Thumbnail */}
                        <div className="aspect-square bg-gray-700 relative">
                            {person.thumbnailUrl ? (
                                <img
                                    src={person.thumbnailUrl}
                                    alt={person.name || `Person ${person.id}`}
                                    className="object-cover w-full h-full"
                                />
                            ) : (
                                <div className="flex items-center justify-center w-full h-full">
                                    <User size={48} className="text-gray-500" />
                                </div>
                            )}

                            {/* Face count badge */}
                            <div className="absolute top-2 right-2 bg-black bg-opacity-70 text-white text-xs px-2 py-1 rounded">
                                {person.face_count}
                            </div>
                        </div>

                        {/* Name */}
                        <div className="p-3">
                            {editingPersonId === person.id ? (
                                <div className="flex gap-1" onClick={(e) => e.stopPropagation()}>
                                    <input
                                        type="text"
                                        value={editName}
                                        onChange={(e) => setEditName(e.target.value)}
                                        onKeyPress={(e) => {
                                            if (e.key === 'Enter') {
                                                handleSaveName(person.id);
                                            } else if (e.key === 'Escape') {
                                                handleCancelEdit();
                                            }
                                        }}
                                        className="flex-1 px-2 py-1 text-sm bg-gray-700 text-white border border-gray-600 rounded"
                                        autoFocus
                                    />
                                    <button
                                        onClick={() => handleSaveName(person.id)}
                                        className="p-1 bg-green-600 hover:bg-green-500 rounded"
                                        title="Save"
                                    >
                                        <Check size={16} />
                                    </button>
                                    <button
                                        onClick={handleCancelEdit}
                                        className="p-1 bg-red-600 hover:bg-red-500 rounded"
                                        title="Cancel"
                                    >
                                        <X size={16} />
                                    </button>
                                </div>
                            ) : (
                                <div className="flex items-center justify-between">
                                    <span className="text-sm text-gray-200 truncate flex-1">
                                        {person.name || `Person ${person.id}`}
                                    </span>
                                    <button
                                        onClick={(e) => {
                                            e.stopPropagation();
                                            handleEditName(person.id, person.name);
                                        }}
                                        className="p-1 opacity-0 group-hover:opacity-100 hover:bg-gray-700 rounded transition-opacity"
                                        title="Edit name"
                                    >
                                        <Edit2 size={14} className="text-gray-400" />
                                    </button>
                                </div>
                            )}
                        </div>
                    </div>
                ))}
            </div>
        </div>
    );
});
