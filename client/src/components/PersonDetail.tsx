import { observer } from "mobx-react-lite";
import { useStore } from "../stores/RootStore";
import { ArrowLeft } from "lucide-react";
import { runInAction } from "mobx";
import { useNavigate } from "react-router-dom";
import { MediaLightbox } from "./MediaLightbox";

export const PersonDetail = observer(() => {
    const { personStore, mediaStore } = useStore();

    const navigate = useNavigate();

    if (!personStore.selectedPerson) {
        return null;
    }

    const person = personStore.selectedPerson;

    const handleImageClick = (index: number) => {
        // Populate mediaStore with person's images so lightbox works
        runInAction(() => {
            mediaStore.images = personStore.personImages.map(img => ({
                hash: img.hash,
                name: img.name,
                created_at: img.created_at,
                device_id: img.deviceid,
                // Reuse existing blob URL from personStore
                thumbnailUrl: img.thumbnailUrl || '',
                place: img.place,
                starred: img.starred
            }));
        });

        mediaStore.openMediaLightbox(index, 'images');
    };

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
                <div>
                    <h2 className="text-2xl font-bold text-gray-200">
                        {person.name || `Person ${person.id}`}
                    </h2>
                    <p className="text-sm text-gray-400">
                        {person.face_count} photo{person.face_count !== 1 ? 's' : ''}
                    </p>
                </div>
            </div>

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
                    {personStore.personImages.map((image, index) => (
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

                            {/* Confidence badge */}
                            <div className="absolute top-2 left-2 bg-black bg-opacity-70 text-white text-xs px-2 py-1 rounded">
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
                    ))}
                </div>
            )}

            {/* Lightbox for viewing full images */}
            {mediaStore.selectedMediaIndex !== null && <MediaLightbox />}
        </div>
    );
});
