import React from "react";
import { observer } from "mobx-react-lite";
import { Bot, CheckCircle2, AlertCircle, Cpu } from "lucide-react";
import { useStore } from "../../stores/RootStore";

export const AiModelsViewer: React.FC = observer(() => {
    const { systemStore } = useStore();
    const aiData = systemStore.aiModels;

    if (!aiData || aiData.models.length === 0) {
        return (
            <div className="bg-gray-800 shadow-xl rounded-xl p-8 border border-gray-700">
                <h2 className="text-xl font-bold text-gray-100 mb-4 flex items-center">
                    <Bot className="w-6 h-6 text-purple-400 mr-3" /> AI Inference Engine & Models
                </h2>
                <div className="text-gray-500 italic">Querying AI model status...</div>
            </div>
        );
    }

    return (
        <div className="bg-gray-800 shadow-xl rounded-xl p-8 border border-gray-700">
            <div className="flex flex-wrap items-center justify-between gap-4 mb-6">
                <div>
                    <h2 className="text-xl font-bold text-gray-100 flex items-center">
                        <Bot className="w-6 h-6 text-purple-400 mr-3" /> AI Inference Engine & Models
                    </h2>
                    <p className="text-xs text-gray-400 mt-1">
                        Active neural network backends loaded in gRPC sidecar
                    </p>
                </div>
                <div className="flex items-center gap-2 px-3 py-1.5 rounded-lg bg-gray-900/80 border border-gray-700 text-xs font-mono text-gray-300">
                    <Cpu className="w-3.5 h-3.5 text-indigo-400" />
                    <span>Device: <strong className="text-indigo-300">{aiData.device}</strong></span>
                    <span className="text-gray-500">|</span>
                    <span className={`inline-flex items-center gap-1 font-semibold ${aiData.status === "healthy" ? "text-green-400" : "text-amber-400"}`}>
                        {aiData.status === "healthy" ? <CheckCircle2 className="w-3.5 h-3.5" /> : <AlertCircle className="w-3.5 h-3.5" />}
                        {aiData.status.toUpperCase()}
                    </span>
                </div>
            </div>

            <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
                {aiData.models.map((m) => {
                    const isWorking = m.loaded && m.status === "active";
                    return (
                        <div
                            key={m.id}
                            className={`p-4 rounded-xl border transition-all duration-200 ${
                                isWorking
                                    ? "bg-gray-900/60 border-gray-700 hover:border-purple-500/50"
                                    : "bg-gray-900/30 border-gray-800 opacity-60"
                            }`}
                        >
                            <div className="flex items-start justify-between gap-2 mb-2">
                                <span className="font-semibold text-gray-200 text-sm">{m.name}</span>
                                <span
                                    className={`px-2 py-0.5 rounded-full text-xs font-medium inline-flex items-center gap-1 ${
                                        isWorking
                                            ? "bg-emerald-950/80 text-emerald-300 border border-emerald-700/50"
                                            : "bg-gray-800 text-gray-400 border border-gray-700"
                                    }`}
                                >
                                    <span className={`w-1.5 h-1.5 rounded-full ${isWorking ? "bg-emerald-400 animate-pulse" : "bg-gray-500"}`} />
                                    {isWorking ? "LOADED & WORKING" : "STANDBY"}
                                </span>
                            </div>

                            <div className="text-xs text-gray-400 mb-2 font-mono truncate" title={m.model_id}>
                                {m.model_id}
                            </div>

                            <div className="flex items-center justify-between text-xs text-gray-400 pt-2 border-t border-gray-800">
                                <span className="text-gray-400">Task: <span className="text-gray-300 font-medium">{m.task}</span></span>
                                {m.dim && (
                                    <span className="font-mono text-purple-300 bg-purple-950/40 px-1.5 py-0.5 rounded border border-purple-800/40">
                                        {m.dim}d
                                    </span>
                                )}
                            </div>
                        </div>
                    );
                })}
            </div>
        </div>
    );
});
