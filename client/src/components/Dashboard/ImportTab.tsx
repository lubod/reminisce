import { Upload, Folder } from "lucide-react";

interface ImportTabProps {
    onOpenBrowserImport: () => void;
    onOpenServerImport: () => void;
}

export function ImportTab({ onOpenBrowserImport, onOpenServerImport }: ImportTabProps) {
    return (
        <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
            <div className="bg-gray-800 shadow-lg rounded-xl p-8 border border-gray-700">
                <div className="flex items-center gap-4 mb-6">
                    <div className="p-3 bg-blue-900/30 rounded-lg"><Upload className="w-8 h-8 text-blue-400" /></div>
                    <div>
                        <h2 className="text-xl font-bold text-gray-100">Browser Upload</h2>
                        <p className="text-gray-400 text-sm">Upload directly from this device.</p>
                    </div>
                </div>
                <button
                    onClick={onOpenBrowserImport}
                    className="w-full py-4 bg-blue-600 hover:bg-blue-700 text-white font-bold rounded-lg shadow-lg active:scale-[0.98] transition-all"
                >
                    Select Files
                </button>
            </div>
            <div className="bg-gray-800 shadow-lg rounded-xl p-8 border border-gray-700">
                <div className="flex items-center gap-4 mb-6">
                    <div className="p-3 bg-indigo-900/30 rounded-lg"><Folder className="w-8 h-8 text-indigo-400" /></div>
                    <div>
                        <h2 className="text-xl font-bold text-gray-100">Server Import</h2>
                        <p className="text-gray-400 text-sm">Scan a folder on the server's disk.</p>
                    </div>
                </div>
                <button
                    onClick={onOpenServerImport}
                    className="w-full py-4 bg-indigo-600 hover:bg-indigo-700 text-white font-bold rounded-lg shadow-lg active:scale-[0.98] transition-all"
                >
                    Choose Directory
                </button>
            </div>
        </div>
    );
}
