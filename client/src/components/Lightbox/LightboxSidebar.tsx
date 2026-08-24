import React from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { Tag, X, Plus } from "lucide-react";
import type { MediaItem, ImageMetadata } from "../../stores/MediaStore";
import type { Label } from "../../stores/LabelStore";

interface LightboxSidebarProps {
    selectedMedia: MediaItem;
    metadata: ImageMetadata | null;
    activeTab: "info" | "description" | "exif" | "labels";
    setActiveTab: (tab: "info" | "description" | "exif" | "labels") => void;
    mediaLabels: Label[];
    availableLabels: Label[];
    showNewLabelInput: boolean;
    setShowNewLabelInput: (show: boolean) => void;
    newLabelName: string;
    setNewLabelName: (name: string) => void;
    handleAddLabel: (labelId: number) => void;
    handleRemoveLabel: (labelId: number) => void;
    handleCreateAndAddLabel: () => void;
}

export const LightboxSidebar: React.FC<LightboxSidebarProps> = ({
    selectedMedia,
    metadata,
    activeTab,
    setActiveTab,
    mediaLabels,
    availableLabels,
    showNewLabelInput,
    setShowNewLabelInput,
    newLabelName,
    setNewLabelName,
    handleAddLabel,
    handleRemoveLabel,
    handleCreateAndAddLabel,
}) => {
    const isVideo = selectedMedia.media_type === "video";

    const formatFileSize = (bytes: number): string => {
        if (bytes >= 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
        if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
        if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)} KB`;
        return `${bytes} B`;
    };

    const formatDate = (dateString: string) => {
        const date = new Date(dateString);
        return date.toLocaleString("en-US", {
            year: "numeric",
            month: "long",
            day: "numeric",
            hour: "2-digit",
            minute: "2-digit",
        });
    };

    const formatExifData = (exifJson: string) => {
        try {
            const exif = JSON.parse(exifJson);
            const formatted: { [key: string]: unknown } = {};

            const strip = (v: unknown): string =>
                typeof v === "string" ? v.replace(/^"|"$/g, "").trim() : String(v);

            const leadNum = (v: unknown): number | null => {
                const n = parseFloat(String(v));
                return isNaN(n) ? null : n;
            };

            if (exif.Make) formatted["Camera Make"] = strip(exif.Make);
            if (exif.Model) formatted["Camera Model"] = strip(exif.Model);
            if (exif.LensModel) formatted["Lens"] = strip(exif.LensModel);

            if (exif.ExposureTime) {
                if (typeof exif.ExposureTime === "number") {
                    formatted["Shutter Speed"] = exif.ExposureTime >= 1
                        ? `${exif.ExposureTime}s`
                        : `1/${Math.round(1 / exif.ExposureTime)}s`;
                } else {
                    formatted["Shutter Speed"] = String(exif.ExposureTime).replace(/ s$/, "");
                }
            }
            if (exif.FNumber) {
                const fn_ = String(exif.FNumber);
                formatted["Aperture"] = fn_.startsWith("f/") ? fn_ : `f/${fn_}`;
            }
            const isoVal = exif.ISO ?? exif.ISOSpeedRatings ?? exif.ISOSpeed ?? exif.PhotographicSensitivity;
            if (isoVal != null) formatted["ISO"] = isoVal;

            if (exif.FocalLength) {
                const n = leadNum(exif.FocalLength);
                formatted["Focal Length"] = n != null ? `${n}mm` : String(exif.FocalLength);
            }
            if (exif.FocalLengthIn35mmFilm) {
                const n = leadNum(exif.FocalLengthIn35mmFilm);
                formatted["Focal Length (35mm equiv.)"] = n != null ? `${n}mm` : String(exif.FocalLengthIn35mmFilm);
            }

            const imgW = leadNum(exif.PixelXDimension ?? exif.ImageWidth);
            const imgH = leadNum(exif.PixelYDimension ?? exif.ImageHeight ?? exif.ImageLength);
            if (imgW && imgH) formatted["Resolution"] = `${imgW} × ${imgH}`;

            if (exif.Orientation) {
                const orientations: { [key: number]: string } = {
                    1: "Normal", 2: "Flipped horizontal", 3: "Rotated 180°",
                    4: "Flipped vertical", 5: "Rotated 90° CCW + flip",
                    6: "Rotated 90° CW", 7: "Rotated 90° CW + flip", 8: "Rotated 90° CCW",
                };
                const orientationStrings: { [key: string]: number } = {
                    "row 0 at top and column 0 at left": 1,
                    "row 0 at top and column 0 at right": 2,
                    "row 0 at bottom and column 0 at right": 3,
                    "row 0 at bottom and column 0 at left": 4,
                    "row 0 at left and column 0 at top": 5,
                    "row 0 at right and column 0 at top": 6,
                    "row 0 at right and column 0 at bottom": 7,
                    "row 0 at left and column 0 at bottom": 8,
                };
                const ori = typeof exif.Orientation === "number"
                    ? exif.Orientation
                    : (orientationStrings[String(exif.Orientation)] ?? parseInt(exif.Orientation, 10));
                formatted["Orientation"] = orientations[ori] || String(exif.Orientation);
            }

            if (exif.DateTime) formatted["Date Taken"] = exif.DateTime;
            if (exif.DateTimeOriginal) formatted["Date Original"] = exif.DateTimeOriginal;
            if (exif.GPSLatitude && exif.GPSLongitude) {
                formatted["GPS"] = `${exif.GPSLatitude}, ${exif.GPSLongitude}`;
            }

            return { formatted, raw: exif };
        } catch {
            return { formatted: {}, raw: {} };
        }
    };

    return (
        <div className="bg-gray-900 bg-opacity-80 text-white p-4 rounded h-80 flex flex-col animate-in fade-in slide-in-from-bottom-4 duration-300">
            <div className="mb-3">
                <div className="text-lg font-semibold truncate">{selectedMedia.name}</div>
            </div>

            <div className="flex gap-2 mb-4 border-b border-gray-700">
                <button
                    onClick={() => setActiveTab("info")}
                    className={`px-4 py-2 text-sm font-medium transition-colors ${
                        activeTab === "info" ? "text-white border-b-2 border-blue-500" : "text-gray-400 hover:text-gray-200"
                    }`}
                >
                    Info
                </button>
                {!isVideo && (
                    <>
                        <button
                            onClick={() => setActiveTab("description")}
                            className={`px-4 py-2 text-sm font-medium transition-colors ${
                                activeTab === "description" ? "text-white border-b-2 border-blue-500" : "text-gray-400 hover:text-gray-200"
                            }`}
                        >
                            Description
                        </button>
                        <button
                            onClick={() => setActiveTab("exif")}
                            className={`px-4 py-2 text-sm font-medium transition-colors ${
                                activeTab === "exif" ? "text-white border-b-2 border-blue-500" : "text-gray-400 hover:text-gray-200"
                            }`}
                        >
                            EXIF
                        </button>
                    </>
                )}
                <button
                    onClick={() => setActiveTab("labels")}
                    className={`px-4 py-2 text-sm font-medium transition-colors flex items-center gap-1 ${
                        activeTab === "labels" ? "text-white border-b-2 border-blue-500" : "text-gray-400 hover:text-gray-200"
                    }`}
                >
                    <Tag size={16} />
                    Labels
                </button>
            </div>

            <div className="overflow-y-auto flex-1" onWheel={(e) => e.stopPropagation()}>
                {activeTab === "info" && (
                    <div className="grid grid-cols-1 md:grid-cols-2 gap-4 text-sm">
                        <div>
                            <div className="text-gray-400 text-xs mb-1">Date & Time</div>
                            <div>{formatDate(selectedMedia.created_at)}</div>
                        </div>
                        <div>
                            <div className="text-gray-400 text-xs mb-1">Location</div>
                            <div className="truncate">{selectedMedia.place || "Unknown"}</div>
                        </div>
                        {selectedMedia.device_id && (
                            <div className="md:col-span-2">
                                <div className="text-gray-400 text-xs mb-1">Device ID</div>
                                <div className="font-mono text-xs">{selectedMedia.device_id}</div>
                            </div>
                        )}
                        <div className="md:col-span-2">
                            <div className="text-gray-400 text-xs mb-1">Type</div>
                            <div className="capitalize">{selectedMedia.media_type || (isVideo ? "Video" : "Image")}</div>
                        </div>
                        {selectedMedia.file_size_bytes != null && (
                            <div>
                                <div className="text-gray-400 text-xs mb-1">File Size</div>
                                <div>{formatFileSize(selectedMedia.file_size_bytes)}</div>
                            </div>
                        )}
                        {metadata?.orientation_label && (
                            <div>
                                <div className="text-gray-400 text-xs mb-1">Orientation</div>
                                <div>{metadata.orientation_label}</div>
                            </div>
                        )}
                    </div>
                )}

                {activeTab === "description" && !isVideo && (
                    <div className="text-sm">
                        {metadata ? (
                            metadata.description ? (
                                <div>
                                    <div className="text-gray-400 text-xs mb-2">AI Description</div>
                                    <div className="text-white prose prose-invert prose-sm max-w-none prose-p:text-gray-200 prose-p:leading-relaxed">
                                        <ReactMarkdown remarkPlugins={[remarkGfm]}>
                                            {metadata.description}
                                        </ReactMarkdown>
                                    </div>
                                </div>
                            ) : (
                                <div className="text-gray-400 italic">No AI description available.</div>
                            )
                        ) : (
                            <div className="text-gray-400">Loading description...</div>
                        )}
                    </div>
                )}

                {activeTab === "exif" && (
                    <div className="text-sm">
                        {metadata ? (
                            metadata.exif ? (() => {
                                const { formatted, raw } = formatExifData(metadata.exif);
                                return (
                                    <div className="space-y-4">
                                        {Object.keys(formatted).length > 0 ? (
                                            <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
                                                {Object.entries(formatted).map(([key, value]) => (
                                                    <div key={key}>
                                                        <div className="text-gray-400 text-xs mb-0.5">{key}</div>
                                                        <div className="text-gray-200 font-mono text-xs">{String(value)}</div>
                                                    </div>
                                                ))}
                                            </div>
                                        ) : (
                                            <pre className="text-xs bg-gray-800 p-2 rounded overflow-auto max-h-40">
                                                {JSON.stringify(raw, null, 2)}
                                            </pre>
                                        )}
                                    </div>
                                );
                            })() : (
                                <div className="text-gray-400 italic">No EXIF data available.</div>
                            )
                        ) : (
                            <div className="text-gray-400">Loading metadata...</div>
                        )}
                    </div>
                )}

                {activeTab === "labels" && (
                    <div className="text-sm space-y-4">
                        <div>
                            <div className="text-gray-400 text-xs mb-2">Current Labels</div>
                            {mediaLabels.length > 0 ? (
                                <div className="flex flex-wrap gap-2">
                                    {mediaLabels.map((label) => (
                                        <span
                                            key={label.id}
                                            className="inline-flex items-center gap-1 px-3 py-1 rounded-full text-sm"
                                            style={{ backgroundColor: label.color + "20", color: label.color }}
                                        >
                                            {label.name}
                                            <button
                                                onClick={() => handleRemoveLabel(label.id)}
                                                className="hover:bg-black hover:bg-opacity-20 rounded-full p-0.5"
                                            >
                                                <X size={14} />
                                            </button>
                                        </span>
                                    ))}
                                </div>
                            ) : (
                                <div className="text-gray-500 italic text-xs">No labels yet</div>
                            )}
                        </div>

                        <div>
                            <div className="text-gray-400 text-xs mb-2">Add Label</div>
                            <div className="flex flex-wrap gap-2">
                                {availableLabels
                                    .filter((l) => !mediaLabels.find((ml) => ml.id === l.id))
                                    .map((label) => (
                                        <button
                                            key={label.id}
                                            onClick={() => handleAddLabel(label.id)}
                                            className="px-3 py-1 rounded-full text-sm hover:opacity-80 transition-opacity"
                                            style={{ backgroundColor: label.color + "20", color: label.color }}
                                        >
                                            + {label.name}
                                        </button>
                                    ))}

                                {!showNewLabelInput ? (
                                    <button
                                        onClick={() => setShowNewLabelInput(true)}
                                        className="px-3 py-1 rounded-full text-sm bg-gray-700 text-gray-300 hover:bg-gray-600 flex items-center gap-1"
                                    >
                                        <Plus size={14} />
                                        New Label
                                    </button>
                                ) : (
                                    <div className="flex items-center gap-2">
                                        <input
                                            type="text"
                                            value={newLabelName}
                                            onChange={(e) => setNewLabelName(e.target.value)}
                                            onKeyPress={(e) => {
                                                if (e.key === "Enter") {
                                                    handleCreateAndAddLabel();
                                                } else if (e.key === "Escape") {
                                                    setShowNewLabelInput(false);
                                                    setNewLabelName("");
                                                }
                                            }}
                                            placeholder="Label name"
                                            className="px-3 py-1 rounded-full text-sm bg-gray-700 text-white border border-gray-600 focus:outline-none focus:border-blue-500"
                                            autoFocus
                                        />
                                        <button
                                            onClick={handleCreateAndAddLabel}
                                            className="px-2 py-1 bg-blue-600 hover:bg-blue-700 text-white rounded text-xs"
                                        >
                                            Add
                                        </button>
                                        <button
                                            onClick={() => {
                                                setShowNewLabelInput(false);
                                                setNewLabelName("");
                                            }}
                                            className="px-2 py-1 bg-gray-600 hover:bg-gray-500 text-white rounded text-xs"
                                        >
                                            Cancel
                                        </button>
                                    </div>
                                )}
                            </div>
                        </div>
                    </div>
                )}
            </div>
        </div>
    );
};
