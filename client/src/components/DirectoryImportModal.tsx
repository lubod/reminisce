import { useState, useRef } from "react";
import type { ChangeEvent } from "react";
import { observer } from "mobx-react-lite";
import { useStore } from "../stores/RootStore";
import { X, Upload, FolderOpen, CheckCircle, AlertCircle, Loader } from "lucide-react";
import axios from "../api/axiosConfig";
import { blake3 } from "hash-wasm";

interface FileWithHash {
    file: File;
    hash: string;
}

interface ImportProgress {
    total: number;
    completed: number;
    failed: number;
    current: string;
    status: 'idle' | 'hashing' | 'checking' | 'uploading' | 'complete' | 'error' | 'cancelled';
}

interface BatchCheckResult {
    exists_for_device: string[];
    needs_upload: string[];
}

interface CheckExistResponse {
    existing_hashes: string[];
}

const IMAGE_EXTENSIONS = ['.jpg', '.jpeg', '.png', '.gif', '.bmp', '.webp'];
const VIDEO_EXTENSIONS = ['.mp4', '.mov', '.avi'];
const MEDIA_EXTENSIONS = [...IMAGE_EXTENSIONS, ...VIDEO_EXTENSIONS];

const isMediaFile = (file: File): boolean => {
    const ext = file.name.toLowerCase().substring(file.name.lastIndexOf('.'));
    return MEDIA_EXTENSIONS.includes(ext);
};

const isVideoFile = (filename: string): boolean => {
    const ext = filename.toLowerCase().substring(filename.lastIndexOf('.'));
    return VIDEO_EXTENSIONS.includes(ext);
};

const calculateHash = async (file: File): Promise<string> => {
    const buffer = await file.arrayBuffer();
    return await blake3(new Uint8Array(buffer));
};

export const DirectoryImportModal = observer(({ onClose }: { onClose: () => void }) => {
    const { labelStore } = useStore();
    const [selectedFiles, setSelectedFiles] = useState<File[]>([]);
    const [directoryName, setDirectoryName] = useState<string>("");
    const [labelName, setLabelName] = useState<string>("");
    const [progress, setProgress] = useState<ImportProgress>({
        total: 0,
        completed: 0,
        failed: 0,
        current: '',
        status: 'idle'
    });
    const [failedFiles, setFailedFiles] = useState<string[]>([]);
    const cancelRef = useRef(false);

    const handleDirectorySelect = (e: ChangeEvent<HTMLInputElement>) => {
        const files = Array.from(e.target.files || []);
        const mediaFiles = files.filter(isMediaFile);
        setSelectedFiles(mediaFiles);

        if (mediaFiles.length > 0) {
            // Extract directory name from first file's webkitRelativePath
            const relativePath = (mediaFiles[0] as any).webkitRelativePath || mediaFiles[0].name;
            const folderName = relativePath.split('/')[0];
            setDirectoryName(folderName);
            setLabelName(folderName);
        }

        setProgress({
            total: mediaFiles.length,
            completed: 0,
            failed: 0,
            current: '',
            status: 'idle'
        });
        setFailedFiles([]);
    };

    const batchCheckMedia = async (files: FileWithHash[]): Promise<BatchCheckResult> => {
        const chunkSize = 100;
        const result: BatchCheckResult = {
            exists_for_device: [],
            needs_upload: []
        };

        for (let i = 0; i < files.length; i += chunkSize) {
            if (cancelRef.current) return result;
            const chunk = files.slice(i, i + chunkSize);

            const images = chunk.filter(f => !isVideoFile(f.file.name));
            const videos = chunk.filter(f => isVideoFile(f.file.name));

            if (images.length > 0) {
                try {
                    const res = await axios.post<CheckExistResponse>('/upload/batch-check-images', {
                        device_id: 'web-client',
                        hashes: images.map(f => f.hash)
                    });
                    result.exists_for_device.push(...res.data.existing_hashes);
                    const existingSet = new Set(res.data.existing_hashes);
                    result.needs_upload.push(...images.filter(f => !existingSet.has(f.hash)).map(f => f.hash));
                } catch (error) {
                    console.error('Failed to batch check images:', error);
                    result.needs_upload.push(...images.map(f => f.hash));
                }
            }

            if (videos.length > 0) {
                try {
                    const res = await axios.post<CheckExistResponse>('/upload/batch-check-videos', {
                        device_id: 'web-client',
                        hashes: videos.map(f => f.hash)
                    });
                    result.exists_for_device.push(...res.data.existing_hashes);
                    const existingSet = new Set(res.data.existing_hashes);
                    result.needs_upload.push(...videos.filter(f => !existingSet.has(f.hash)).map(f => f.hash));
                } catch (error) {
                    console.error('Failed to batch check videos:', error);
                    result.needs_upload.push(...videos.map(f => f.hash));
                }
            }
        }

        return result;
    };

    const uploadFile = async (file: File, hash: string, labelId: number | null): Promise<void> => {
        const isVideo = isVideoFile(file.name);
        const formData = new FormData();
        formData.append('hash', hash);
        formData.append('name', file.name);
        formData.append(isVideo ? 'video' : 'image', file);

        await axios.post(isVideo ? '/upload/video' : '/upload/image', formData);

        if (labelId) {
            try {
                const endpoint = isVideo ? `/videos/${hash}/labels` : `/images/${hash}/labels`;
                await axios.post(endpoint, { label_id: labelId });
            } catch (error) {
                console.error(`Failed to apply label to ${file.name}:`, error);
            }
        }
    };

    const handleImport = async () => {
        if (selectedFiles.length === 0) {
            return;
        }
        
        cancelRef.current = false;

        try {
            // Phase 1: Create or find label
            let labelId: number | null = null;
            if (labelName.trim()) {
                if (cancelRef.current) return;
                setProgress(prev => ({ ...prev, status: 'hashing', current: 'Creating label...' }));
                try {
                    const label = await labelStore.createLabel(labelName.trim());
                    labelId = label.id;
                } catch (error: any) {
                    // Label might already exist (409 conflict)
                    if (error.response?.status === 409) {
                        // Find existing label with same name
                        await labelStore.fetchLabels();
                        const existingLabel = labelStore.labels.find(l => l.name === labelName.trim());
                        if (existingLabel) {
                            labelId = existingLabel.id;
                        }
                    } else {
                        console.error('Failed to create label:', error);
                    }
                }
            }

            // Phase 2: Calculate hashes (parallelized)
            if (cancelRef.current) return;
            setProgress(prev => ({ ...prev, status: 'hashing' }));
            const fileHashes: FileWithHash[] = [];

            const CONCURRENT_HASHES = 4;
            for (let i = 0; i < selectedFiles.length; i += CONCURRENT_HASHES) {
                if (cancelRef.current) {
                    setProgress(prev => ({ ...prev, status: 'cancelled', current: 'Import cancelled' }));
                    return;
                }
                
                const chunk = selectedFiles.slice(i, i + CONCURRENT_HASHES);
                setProgress(prev => ({
                    ...prev,
                    current: `Hashing ${i + 1}-${Math.min(i + CONCURRENT_HASHES, selectedFiles.length)} of ${selectedFiles.length}`
                }));

                const results = await Promise.all(chunk.map(async (file) => {
                    try {
                        const hash = await calculateHash(file);
                        return { file, hash };
                    } catch (error) {
                        console.error(`Failed to hash ${file.name}:`, error);
                        setFailedFiles(prev => [...prev, file.name]);
                        return null;
                    }
                }));

                for (const res of results) {
                    if (res) fileHashes.push(res);
                }
            }

            // Phase 3: Check existing files
            if (cancelRef.current) {
                setProgress(prev => ({ ...prev, status: 'cancelled', current: 'Import cancelled' }));
                return;
            }
            setProgress(prev => ({
                ...prev,
                status: 'checking',
                current: 'Checking for existing files and deduplicating...'
            }));

            const checkResult = await batchCheckMedia(fileHashes);
            if (cancelRef.current) {
                setProgress(prev => ({ ...prev, status: 'cancelled', current: 'Import cancelled' }));
                return;
            }

            // Only upload files not already on the server
            const toUpload = fileHashes.filter(f => checkResult.needs_upload.includes(f.hash));
            const alreadyHandled = fileHashes.length - toUpload.length;

            console.log(`${alreadyHandled} files already exist or were deduplicated, uploading ${toUpload.length} new files`);

            // Phase 4: Upload sequentially
            setProgress(prev => ({
                ...prev,
                status: 'uploading',
                total: toUpload.length,
                completed: 0,
                failed: 0
            }));

            for (let i = 0; i < toUpload.length; i++) {
                if (cancelRef.current) {
                    setProgress(prev => ({ ...prev, status: 'cancelled', current: 'Import cancelled' }));
                    return;
                }
                const { file, hash } = toUpload[i];
                setProgress(prev => ({
                    ...prev,
                    current: `Uploading ${file.name} (${i + 1}/${toUpload.length})`
                }));

                try {
                    await uploadFile(file, hash, labelId);
                    setProgress(prev => ({ ...prev, completed: prev.completed + 1 }));
                } catch (error: any) {
                    console.error(`Failed to upload ${file.name}:`, error);
                    setFailedFiles(prev => [...prev, file.name]);
                    setProgress(prev => ({ ...prev, failed: prev.failed + 1 }));
                }
            }

            setProgress(prev => ({ ...prev, status: 'complete', current: 'Import complete!' }));
        } catch (error) {
            console.error('Import failed:', error);
            setProgress(prev => ({ ...prev, status: 'error', current: 'Import failed' }));
        }
    };


    const isUploading = progress.status === 'hashing' || progress.status === 'checking' || progress.status === 'uploading';
    const canImport = selectedFiles.length > 0 && !isUploading;

    const handleCancel = () => {
        if (isUploading) {
            cancelRef.current = true;
            setProgress(prev => ({ ...prev, current: 'Cancelling...' }));
        } else {
            onClose();
        }
    };

    return (
        <div className="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50">
            <div className="bg-gray-800 rounded-lg shadow-xl max-w-2xl w-full mx-4 max-h-[90vh] overflow-y-auto">
                {/* Header */}
                <div className="flex items-center justify-between p-6 border-b border-gray-700">
                    <h2 className="text-xl font-semibold text-gray-100 flex items-center">
                        <FolderOpen className="w-6 h-6 mr-2" />
                        Import Directory
                    </h2>
                    <button
                        onClick={handleCancel}
                        className="text-gray-400 hover:text-gray-200 transition-colors"
                    >
                        <X className="w-6 h-6" />
                    </button>
                </div>

                {/* Content */}
                <div className="p-6 space-y-6">
                    {/* Directory Selection */}
                    <div>
                        <label className="block text-sm font-medium text-gray-300 mb-2">
                            Select Directory
                        </label>
                        <input
                            type="file"
                            /* @ts-ignore */
                            webkitdirectory=""
                            directory=""
                            multiple
                            onChange={handleDirectorySelect}
                            disabled={isUploading}
                            className="block w-full text-sm text-gray-300
                                file:mr-4 file:py-2 file:px-4
                                file:rounded-md file:border-0
                                file:text-sm file:font-semibold
                                file:bg-blue-600 file:text-white
                                hover:file:bg-blue-700
                                file:cursor-pointer
                                disabled:opacity-50 disabled:cursor-not-allowed"
                        />
                        {selectedFiles.length > 0 && (
                            <p className="mt-2 text-sm text-gray-400">
                                Selected {selectedFiles.length} media file{selectedFiles.length !== 1 ? 's' : ''} from "{directoryName}"
                            </p>
                        )}
                    </div>

                    {/* Label Configuration */}
                    {selectedFiles.length > 0 && (
                        <div>
                            <label className="block text-sm font-medium text-gray-300 mb-2">
                                Label (auto-created from directory name)
                            </label>
                            <input
                                type="text"
                                value={labelName}
                                onChange={(e) => setLabelName(e.target.value)}
                                disabled={isUploading}
                                placeholder="Label name"
                                className="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded-md
                                    text-gray-300 placeholder-gray-500
                                    focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent
                                    disabled:opacity-50 disabled:cursor-not-allowed"
                            />
                            <p className="mt-1 text-xs text-gray-500">
                                All imported files will be tagged with this label
                            </p>
                        </div>
                    )}

                    {/* Progress Display */}
                    {progress.status !== 'idle' && (
                        <div className="bg-gray-700 rounded-lg p-4 space-y-3">
                            <div className="flex items-center justify-between">
                                <span className="text-sm font-medium text-gray-300">
                                    {progress.status === 'hashing' && 'Calculating hashes...'}
                                    {progress.status === 'checking' && 'Checking existing files...'}
                                    {progress.status === 'uploading' && 'Uploading files...'}
                                    {progress.status === 'complete' && 'Complete!'}
                                    {progress.status === 'error' && 'Error occurred'}
                                    {progress.status === 'cancelled' && 'Cancelled'}
                                </span>
                                {isUploading && <Loader className="w-5 h-5 animate-spin text-blue-500" />}
                                {progress.status === 'complete' && <CheckCircle className="w-5 h-5 text-green-500" />}
                                {progress.status === 'error' && <AlertCircle className="w-5 h-5 text-red-500" />}
                                {progress.status === 'cancelled' && <X className="w-5 h-5 text-yellow-500" />}
                            </div>

                            {progress.current && (
                                <p className="text-sm text-gray-400">{progress.current}</p>
                            )}

                            {progress.status === 'uploading' && (
                                <div>
                                    <div className="flex justify-between text-sm text-gray-400 mb-1">
                                        <span>Progress: {progress.completed} / {progress.total}</span>
                                        <span>{Math.round((progress.completed / progress.total) * 100)}%</span>
                                    </div>
                                    <div className="w-full bg-gray-600 rounded-full h-2">
                                        <div
                                            className="bg-blue-600 h-2 rounded-full transition-all duration-300"
                                            style={{ width: `${(progress.completed / progress.total) * 100}%` }}
                                        />
                                    </div>
                                    {progress.failed > 0 && (
                                        <p className="text-sm text-red-400 mt-2">
                                            {progress.failed} file{progress.failed !== 1 ? 's' : ''} failed
                                        </p>
                                    )}
                                </div>
                            )}

                            {progress.status === 'complete' && (
                                <div className="text-sm text-gray-300">
                                    <p>Uploaded {progress.completed} file{progress.completed !== 1 ? 's' : ''} successfully</p>
                                    {progress.failed > 0 && (
                                        <p className="text-red-400">{progress.failed} file{progress.failed !== 1 ? 's' : ''} failed</p>
                                    )}
                                </div>
                            )}
                        </div>
                    )}

                    {/* Failed Files */}
                    {failedFiles.length > 0 && (
                        <div className="bg-red-900/20 border border-red-500/50 rounded-lg p-4">
                            <h3 className="text-sm font-medium text-red-400 mb-2">Failed Files:</h3>
                            <ul className="text-xs text-red-300 space-y-1 max-h-32 overflow-y-auto">
                                {failedFiles.map((filename, index) => (
                                    <li key={index}>• {filename}</li>
                                ))}
                            </ul>
                        </div>
                    )}
                </div>

                {/* Footer */}
                <div className="flex justify-end gap-3 p-6 border-t border-gray-700">
                    <button
                        onClick={handleCancel}
                        className="px-4 py-2 text-gray-300 bg-gray-700 hover:bg-gray-600
                            rounded-md transition-colors"
                    >
                        {progress.status === 'complete' ? 'Close' : (isUploading ? 'Cancel' : 'Cancel')}
                    </button>
                    <button
                        onClick={handleImport}
                        disabled={!canImport}
                        className="flex items-center px-4 py-2 bg-blue-600 hover:bg-blue-700
                            text-white rounded-md transition-colors
                            disabled:opacity-50 disabled:cursor-not-allowed"
                    >
                        <Upload className="w-4 h-4 mr-2" />
                        Import
                    </button>
                </div>
            </div>
        </div>
    );
});
