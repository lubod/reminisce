import { Brain } from "lucide-react";

interface SettingsTabProps {
    loaded: boolean;
    enableAiDescriptions: boolean;
    setEnableAiDescriptions: (v: boolean) => void;
    enableEmbeddings: boolean;
    setEnableEmbeddings: (v: boolean) => void;
    enableFaceDetection: boolean;
    setEnableFaceDetection: (v: boolean) => void;
    onSave: () => void;
}

export function SettingsTab({
    loaded,
    enableAiDescriptions,
    setEnableAiDescriptions,
    enableEmbeddings,
    setEnableEmbeddings,
    enableFaceDetection,
    setEnableFaceDetection,
    onSave,
}: SettingsTabProps) {
    return (
        <div className="bg-gray-800 shadow-xl rounded-xl p-8 border border-gray-700">
            <h2 className="text-xl font-bold text-gray-100 mb-8 flex items-center"><Brain className="w-6 h-6 text-pink-400 mr-3" /> AI & Processing</h2>
            {loaded ? (
                <div className="space-y-10">
                    <div className="grid grid-cols-1 md:grid-cols-3 gap-8">
                        <label className="flex items-start cursor-pointer p-4 bg-gray-900/30 rounded-xl border border-gray-700 hover:border-blue-500/50 transition-colors">
                            <input type="checkbox" checked={enableAiDescriptions} onChange={(e) => setEnableAiDescriptions(e.target.checked)} className="w-6 h-6 mt-1 text-blue-600 bg-gray-700 border-gray-600 rounded focus:ring-blue-500" />
                            <div className="ml-4">
                                <span className="text-gray-200 font-bold block">AI Captions</span>
                                <p className="text-[10px] text-gray-500 mt-1 uppercase font-black">Automatic descriptions for images</p>
                            </div>
                        </label>
                        <label className="flex items-start cursor-pointer p-4 bg-gray-900/30 rounded-xl border border-gray-700 hover:border-purple-500/50 transition-colors">
                            <input type="checkbox" checked={enableEmbeddings} onChange={(e) => setEnableEmbeddings(e.target.checked)} className="w-6 h-6 mt-1 text-purple-600 bg-gray-700 border-gray-600 rounded focus:ring-purple-500" />
                            <div className="ml-4">
                                <span className="text-gray-200 font-bold block">Semantic Index</span>
                                <p className="text-[10px] text-gray-500 mt-1 uppercase font-black">Enable natural language search</p>
                            </div>
                        </label>
                        <label className="flex items-start cursor-pointer p-4 bg-gray-900/30 rounded-xl border border-gray-700 hover:border-orange-500/50 transition-colors">
                            <input type="checkbox" checked={enableFaceDetection} onChange={(e) => setEnableFaceDetection(e.target.checked)} className="w-6 h-6 mt-1 text-orange-600 bg-gray-700 border-gray-600 rounded focus:ring-orange-500" />
                            <div className="ml-4">
                                <span className="text-gray-200 font-bold block">Face Grouping</span>
                                <p className="text-[10px] text-gray-500 mt-1 uppercase font-black">Identify and cluster people</p>
                            </div>
                        </label>
                    </div>
                    <div className="flex justify-end pt-6 border-t border-gray-700">
                        <button onClick={onSave} className="px-10 py-4 bg-blue-600 hover:bg-blue-700 text-white font-black rounded-xl shadow-xl shadow-blue-500/20 transition-all active:scale-95">
                            SAVE PREFERENCES
                        </button>
                    </div>
                </div>
            ) : <div className="text-gray-500 italic">Syncing settings from vault...</div>}
        </div>
    );
}
