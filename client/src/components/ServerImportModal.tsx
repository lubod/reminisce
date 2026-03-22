import { useState, useEffect, useRef } from "react";
import { observer } from "mobx-react-lite";
import { X, Upload, Folder, CheckCircle, AlertCircle, Loader } from "lucide-react";
import axios from "../api/axiosConfig";

interface ImportJob {
    status: "running" | "done" | "failed";
    scanned: number;
    imported: number;
    failed: number;
    errors: string[];
}

export const ServerImportModal = observer(({ onClose }: { onClose: () => void }) => {
    const [path, setPath] = useState<string>("");
    const [recursive, setRecursive] = useState<boolean>(true);
    const [addLabels, setAddLabels] = useState<boolean>(false);
    const [labelMode, setLabelMode] = useState<"root" | "subdir" | "path" | "components">("subdir");
    const [isLoading, setIsLoading] = useState<boolean>(false);
    const [job, setJob] = useState<ImportJob | null>(null);
    const [error, setError] = useState<string | null>(null);
    const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);

    useEffect(() => {
        return () => { if (pollRef.current) clearInterval(pollRef.current); };
    }, []);

    const handleImport = async () => {
        if (!path.trim()) return;

        setIsLoading(true);
        setError(null);
        setJob(null);

        try {
            const response = await axios.post<{ job_id: string }>('/import_directory', {
                path: path.trim(),
                recursive,
                label_mode: addLabels ? labelMode : "none",
            });

            const jobId = response.data.job_id;
            pollRef.current = setInterval(async () => {
                try {
                    const status = await axios.get<ImportJob>(`/import_directory/status/${jobId}`);
                    setJob(status.data);
                    if (status.data.status !== "running") {
                        clearInterval(pollRef.current!);
                        pollRef.current = null;
                        setIsLoading(false);
                    }
                } catch {
                    clearInterval(pollRef.current!);
                    pollRef.current = null;
                    setIsLoading(false);
                }
            }, 2000);
        } catch (err: any) {
            console.error("Import failed:", err);
            setError(err.response?.data?.error || err.message || "An error occurred during import.");
            setIsLoading(false);
        }
    };

    return (
        <div className="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50">
            <div className="bg-gray-800 rounded-lg shadow-xl max-w-lg w-full mx-4">
                {/* Header */}
                <div className="flex items-center justify-between p-6 border-b border-gray-700">
                    <h2 className="text-xl font-semibold text-gray-100 flex items-center">
                        <Folder className="w-6 h-6 mr-2" />
                        Server-Side Import
                    </h2>
                    <button
                        onClick={onClose}
                        className="text-gray-400 hover:text-gray-200 transition-colors"
                        disabled={isLoading}
                    >
                        <X className="w-6 h-6" />
                    </button>
                </div>

                {/* Content */}
                <div className="p-6 space-y-6">
                    <div>
                        <label className="block text-sm font-medium text-gray-300 mb-2">
                            Server Directory Path
                        </label>
                        <input
                            type="text"
                            value={path}
                            onChange={(e) => setPath(e.target.value)}
                            disabled={isLoading}
                            placeholder="/path/to/media"
                            className="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded-md
                                text-gray-300 placeholder-gray-500
                                focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent
                                disabled:opacity-50 disabled:cursor-not-allowed"
                        />
                        <p className="mt-1 text-xs text-gray-500">
                            Absolute path on the server filesystem.
                        </p>
                    </div>

                    <div className="flex items-center">
                        <input
                            type="checkbox"
                            id="recursive"
                            checked={recursive}
                            onChange={(e) => setRecursive(e.target.checked)}
                            disabled={isLoading}
                            className="w-4 h-4 text-blue-600 bg-gray-700 border-gray-600 rounded focus:ring-blue-500"
                        />
                        <label htmlFor="recursive" className="ml-2 text-sm text-gray-300">
                            Scan recursively
                        </label>
                    </div>

                    <div className="space-y-2">
                        <div className="flex items-center">
                            <input
                                type="checkbox"
                                id="addLabels"
                                checked={addLabels}
                                onChange={(e) => setAddLabels(e.target.checked)}
                                disabled={isLoading}
                                className="w-4 h-4 text-blue-600 bg-gray-700 border-gray-600 rounded focus:ring-blue-500"
                            />
                            <label htmlFor="addLabels" className="ml-2 text-sm text-gray-300">
                                Add labels from directory names
                            </label>
                        </div>
                        {addLabels && (
                            <div className="ml-6 flex flex-col gap-2 mt-1">
                                {(
                                    [
                                        { value: "root",       label: "Root directory",        example: 'e.g. "photos"' },
                                        { value: "subdir",     label: "Immediate parent folder", example: 'e.g. "beach"' },
                                        { value: "path",       label: "Full relative path",     example: 'e.g. "2023/vacation/beach"' },
                                        { value: "components", label: "Each folder level",      example: 'e.g. "2023" + "vacation" + "beach"' },
                                    ] as const
                                ).map(({ value, label, example }) => (
                                    <label key={value} className="flex items-start gap-2 cursor-pointer">
                                        <input
                                            type="radio"
                                            name="labelMode"
                                            value={value}
                                            checked={labelMode === value}
                                            onChange={() => setLabelMode(value)}
                                            disabled={isLoading}
                                            className="mt-0.5 text-blue-600"
                                        />
                                        <span>
                                            <span className="text-sm text-gray-300">{label}</span>
                                            <span className="ml-2 text-xs text-gray-500">{example}</span>
                                        </span>
                                    </label>
                                ))}
                            </div>
                        )}
                    </div>

                    {/* Status */}
                    {isLoading && (
                        <div className="space-y-2">
                            <div className="flex items-center text-blue-400">
                                <Loader className="w-5 h-5 animate-spin mr-2 flex-shrink-0" />
                                <span className="text-sm">
                                    {job ? "Importing..." : "Starting import..."}
                                </span>
                            </div>
                            {job && job.scanned > 0 && (
                                <div className="bg-gray-700 rounded-lg p-3 text-sm text-gray-300 space-y-1">
                                    <div>Scanned: {job.scanned} files</div>
                                    <div>Imported: {job.imported}</div>
                                    {job.failed > 0 && <div className="text-red-400">Failed: {job.failed}</div>}
                                </div>
                            )}
                        </div>
                    )}

                    {error && (
                        <div className="bg-red-900/20 border border-red-500/50 rounded-lg p-4 flex items-start text-red-200">
                            <AlertCircle className="w-5 h-5 mr-2 flex-shrink-0 mt-0.5" />
                            <p className="text-sm">{error}</p>
                        </div>
                    )}

                    {job && job.status !== "running" && (
                        <div className="bg-gray-700 rounded-lg p-4 space-y-2">
                            <h3 className="text-sm font-semibold text-gray-200 mb-2 flex items-center">
                                {job.status === "done"
                                    ? <CheckCircle className="w-5 h-5 text-green-500 mr-2" />
                                    : <AlertCircle className="w-5 h-5 text-red-500 mr-2" />
                                }
                                Import {job.status === "done" ? "Completed" : "Failed"}
                            </h3>
                            <div className="grid grid-cols-3 gap-2 text-center">
                                <div className="bg-gray-800 p-2 rounded">
                                    <div className="text-lg font-bold text-gray-100">{job.scanned}</div>
                                    <div className="text-xs text-gray-400">Scanned</div>
                                </div>
                                <div className="bg-gray-800 p-2 rounded">
                                    <div className="text-lg font-bold text-green-400">{job.imported}</div>
                                    <div className="text-xs text-gray-400">Imported</div>
                                </div>
                                <div className="bg-gray-800 p-2 rounded">
                                    <div className="text-lg font-bold text-red-400">{job.failed}</div>
                                    <div className="text-xs text-gray-400">Failed</div>
                                </div>
                            </div>
                            {job.errors.length > 0 && (
                                <div className="mt-2">
                                    <p className="text-xs text-red-400 font-semibold mb-1">Errors:</p>
                                    <ul className="text-xs text-red-300 list-disc list-inside max-h-24 overflow-y-auto">
                                        {job.errors.map((e, i) => <li key={i}>{e}</li>)}
                                    </ul>
                                </div>
                            )}
                        </div>
                    )}
                </div>

                {/* Footer */}
                <div className="flex justify-end gap-3 p-6 border-t border-gray-700">
                    <button
                        onClick={onClose}
                        className="px-4 py-2 text-gray-300 bg-gray-700 hover:bg-gray-600
                            rounded-md transition-colors"
                        disabled={isLoading}
                    >
                        Close
                    </button>
                    <button
                        onClick={handleImport}
                        disabled={isLoading || !path.trim()}
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
