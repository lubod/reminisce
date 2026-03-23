import { useState } from "react";
import { observer } from "mobx-react-lite";
import { useStore } from "../stores/RootStore";
import { ArrowLeft, Star, Merge, X, Check, Search, User } from "lucide-react";
import { runInAction } from "mobx";
import { useNavigate } from "react-router-dom";
import { MediaLightbox } from "./MediaLightbox";

export const PersonDetail = observer(() => {
    const { personStore, mediaStore } = useStore();
    const navigate = useNavigate();

    const [showMergePanel, setShowMergePanel] = useState(false);
    const [mergeSearch, setMergeSearch] = useState("");
    const [mergeTargetId, setMergeTargetId] = useState<number | null>(null);
    const [isMerging, setIsMerging] = useState(false);
    const [settingCoverId, setSettingCoverId] = useState<number | null>(null);

    if (!personStore.selectedPerson) return null;

    const person = personStore.selectedPerson;

    const handleImageClick = (index: number) => {
        runInAction(() => {
            mediaStore.images = personStore.personImages.map(img => ({
                hash: img.hash,
                name: img.name,
                created_at: img.created_at,
                device_id: img.deviceid,
                thumbnailUrl: img.fullThumbnailUrl || img.thumbnailUrl || '',
                place: img.place,
                starred: img.starred
            }));
        });
        mediaStore.openMediaLightbox(index, 'images');
    };

    const handleSetCover = async (e: React.MouseEvent, faceId: number) => {
        e.stopPropagation();
        setSettingCoverId(faceId);
        await personStore.setRepresentativeFace(person.id, faceId);
        setSettingCoverId(null);
    };

    const handleMerge = async () => {
        if (!mergeTargetId || mergeTargetId === person.id) return;
        setIsMerging(true);
        // merge source=current into target (target keeps its id and name)
        await personStore.mergePersons(person.id, mergeTargetId);
        setIsMerging(false);
        navigate("/people");
    };

    const otherPersons = personStore.persons.filter(p => p.id !== person.id);
    const filteredPersons = mergeSearch.trim()
        ? otherPersons.filter(p =>
            (p.name || `Person ${p.id}`).toLowerCase().includes(mergeSearch.toLowerCase())
          )
        : otherPersons;

    return (
        <div>
            {/* Header */}
            <div className="mb-6 flex items-center gap-4">
                <button
                    onClick={() => navigate("/people")}
                    className="p-2 hover:bg-gray-700 rounded-lg transition-colors"
                    title="Back to all persons"
                >
                    <ArrowLeft size={24} className="text-gray-400" />
                </button>
                <div className="flex-1">
                    <h2 className="text-2xl font-bold text-gray-200">
                        {person.name || `Person ${person.id}`}
                    </h2>
                    <p className="text-sm text-gray-400">
                        {person.face_count} photo{person.face_count !== 1 ? 's' : ''}
                    </p>
                </div>
                <button
                    onClick={() => { setShowMergePanel(!showMergePanel); setMergeTargetId(null); setMergeSearch(""); }}
                    className={`flex items-center gap-2 px-4 py-2 rounded-lg text-sm font-medium transition-colors ${showMergePanel ? 'bg-purple-600 text-white' : 'bg-gray-700 hover:bg-gray-600 text-gray-300'}`}
                    title="Merge with another person"
                >
                    <Merge size={16} />
                    Merge
                </button>
            </div>

            {/* Merge Panel */}
            {showMergePanel && (
                <div className="mb-6 bg-gray-800 border border-purple-700/50 rounded-xl p-5">
                    <div className="flex items-center justify-between mb-4">
                        <h3 className="text-sm font-semibold text-purple-300 flex items-center gap-2">
                            <Merge size={15} />
                            Merge "{person.name || `Person ${person.id}`}" into another person
                        </h3>
                        <button onClick={() => setShowMergePanel(false)} className="text-gray-500 hover:text-gray-300">
                            <X size={18} />
                        </button>
                    </div>
                    <p className="text-xs text-gray-400 mb-4">
                        All faces from this person will be moved to the selected person. This person will be deleted.
                    </p>

                    {/* Search */}
                    <div className="relative mb-3">
                        <Search size={14} className="absolute left-3 top-1/2 -translate-y-1/2 text-gray-500" />
                        <input
                            type="text"
                            placeholder="Search persons..."
                            value={mergeSearch}
                            onChange={e => setMergeSearch(e.target.value)}
                            className="w-full pl-8 pr-3 py-2 bg-gray-700 border border-gray-600 rounded-lg text-sm text-gray-200 placeholder-gray-500 focus:outline-none focus:ring-1 focus:ring-purple-500"
                        />
                    </div>

                    {/* Person list */}
                    <div className="max-h-52 overflow-y-auto grid grid-cols-3 sm:grid-cols-5 gap-2 mb-4">
                        {filteredPersons.map(p => (
                            <button
                                key={p.id}
                                onClick={() => setMergeTargetId(mergeTargetId === p.id ? null : p.id)}
                                className={`relative rounded-lg overflow-hidden border-2 transition-all ${
                                    mergeTargetId === p.id
                                        ? 'border-purple-500 ring-2 ring-purple-500/50'
                                        : 'border-transparent hover:border-gray-500'
                                }`}
                            >
                                <div className="aspect-square bg-gray-700">
                                    {p.thumbnailUrl ? (
                                        <img src={p.thumbnailUrl} alt={p.name || ''} className="w-full h-full object-cover" />
                                    ) : (
                                        <div className="flex items-center justify-center w-full h-full">
                                            <User size={24} className="text-gray-500" />
                                        </div>
                                    )}
                                </div>
                                <div className="px-1 py-1 bg-gray-800 text-[10px] text-gray-300 truncate text-center">
                                    {p.name || `Person ${p.id}`}
                                </div>
                                {mergeTargetId === p.id && (
                                    <div className="absolute top-1 right-1 bg-purple-600 rounded-full p-0.5">
                                        <Check size={10} className="text-white" />
                                    </div>
                                )}
                            </button>
                        ))}
                        {filteredPersons.length === 0 && (
                            <p className="col-span-full text-xs text-gray-500 italic py-4 text-center">No persons found</p>
                        )}
                    </div>

                    <button
                        onClick={handleMerge}
                        disabled={!mergeTargetId || isMerging}
                        className="w-full py-2 bg-purple-600 hover:bg-purple-700 disabled:opacity-50 disabled:cursor-not-allowed text-white text-sm font-medium rounded-lg transition-colors"
                    >
                        {isMerging ? "Merging..." : mergeTargetId
                            ? `Merge into "${personStore.persons.find(p => p.id === mergeTargetId)?.name || `Person ${mergeTargetId}`}"`
                            : "Select a person to merge into"}
                    </button>
                </div>
            )}

            {/* Images Grid */}
            {personStore.isLoadingImages ? (
                <div className="flex items-center justify-center h-64">
                    <div className="text-gray-400">Loading images...</div>
                </div>
            ) : personStore.personImages.length === 0 ? (
                <div className="flex items-center justify-center h-64 text-gray-400">
                    <p>No images found for this person</p>
                </div>
            ) : (
                <div className="grid grid-cols-3 xl:grid-cols-6 gap-2">
                    {personStore.personImages.map((image, index) => {
                        const isCurrentCover = image.face_id === person.representative_face_id;
                        return (
                            <div
                                key={`${image.hash}-${index}`}
                                className="relative aspect-square bg-gray-700 rounded-lg overflow-hidden cursor-pointer group"
                                onClick={() => handleImageClick(index)}
                            >
                                {image.thumbnailUrl ? (
                                    <img
                                        src={image.thumbnailUrl}
                                        alt={image.name}
                                        className="object-cover w-full h-full group-hover:opacity-75 transition-opacity"
                                    />
                                ) : (
                                    <div className="flex items-center justify-center w-full h-full text-gray-400">
                                        Loading...
                                    </div>
                                )}

                                {/* Current cover indicator */}
                                {isCurrentCover && (
                                    <div className="absolute top-2 left-2 bg-yellow-500 rounded-full p-1" title="Current cover photo">
                                        <Star size={10} className="text-black fill-black" />
                                    </div>
                                )}

                                {/* Set as cover button */}
                                {!isCurrentCover && (
                                    <button
                                        onClick={(e) => handleSetCover(e, image.face_id)}
                                        disabled={settingCoverId === image.face_id}
                                        className="absolute top-2 left-2 opacity-0 group-hover:opacity-100 bg-black/60 hover:bg-yellow-600 text-white rounded-full p-1 transition-all"
                                        title="Set as cover photo"
                                    >
                                        {settingCoverId === image.face_id
                                            ? <div className="w-2.5 h-2.5 border-2 border-white border-t-transparent rounded-full animate-spin" />
                                            : <Star size={10} />
                                        }
                                    </button>
                                )}

                                {/* Confidence badge */}
                                <div className="absolute top-2 right-2 bg-black bg-opacity-70 text-white text-xs px-2 py-1 rounded">
                                    {(image.confidence * 100).toFixed(1)}%
                                </div>

                                {/* Date overlay */}
                                <div className="absolute bottom-0 left-0 right-0 bg-gradient-to-t from-black to-transparent p-2">
                                    <div className="text-white text-xs truncate">
                                        {new Date(image.created_at).toLocaleDateString('en-US', {
                                            month: 'short',
                                            day: 'numeric',
                                            year: 'numeric'
                                        })}
                                    </div>
                                </div>
                            </div>
                        );
                    })}
                </div>
            )}

            {mediaStore.selectedMediaIndex !== null && <MediaLightbox />}
        </div>
    );
});
