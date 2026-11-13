import { RefreshCw, Shield, X } from "lucide-react";
import type { VerificationResult } from "../../stores/StatsStore";

interface VerifyBackupModalProps {
    isVerifying: boolean;
    verificationResult: VerificationResult | null;
    onClose: () => void;
    onStartVerification: () => void;
}

export function VerifyBackupModal({
    isVerifying,
    verificationResult,
    onClose,
    onStartVerification,
}: VerifyBackupModalProps) {
    return (
        <div
            className="fixed inset-0 bg-black/80 backdrop-blur-xl flex items-center justify-center z-50 p-4 overscroll-contain"
            onClick={onClose}
        >
            <div
                role="dialog"
                aria-modal="true"
                aria-labelledby="verify-modal-title"
                onClick={(e) => e.stopPropagation()}
                className="bg-gray-800 rounded-3xl border border-gray-700 shadow-2xl max-w-2xl w-full overflow-hidden"
            >
                <div className="px-8 py-6 border-b border-gray-700 flex justify-between items-center bg-gray-800/50">
                    <h3 id="verify-modal-title" className="text-xl font-black text-gray-100 flex items-center gap-3"><Shield className="w-6 h-6 text-emerald-400" /> SHARD VERIFICATION</h3>
                    <button onClick={onClose} aria-label="Close modal" className="p-2 hover:bg-gray-700 rounded-full transition-colors"><X className="w-6 h-6 text-gray-400" /></button>
                </div>
                <div className="p-8" aria-live="polite">
                    {isVerifying ? (
                        <div className="flex flex-col items-center justify-center py-16">
                            <RefreshCw className="w-16 h-16 text-emerald-400 animate-spin mb-6" />
                            <p className="text-gray-100 text-2xl font-black tracking-tight">Checking Integrity...</p>
                            <p className="text-gray-500 mt-2 font-bold uppercase text-xs tracking-widest">Scanning network shards</p>
                        </div>
                    ) : verificationResult ? (
                        <div className="space-y-8">
                            <div className="grid grid-cols-2 md:grid-cols-4 gap-6">
                                <div className="bg-emerald-900/20 border border-emerald-500/20 rounded-2xl p-6 text-center">
                                    <div className="text-3xl font-black text-emerald-400">{verificationResult.verified_files}</div>
                                    <div className="text-[10px] text-emerald-500 font-black uppercase mt-1">Full 5/5</div>
                                </div>
                                <div className="bg-yellow-900/20 border border-yellow-500/20 rounded-2xl p-6 text-center">
                                    <div className="text-3xl font-black text-yellow-400">{verificationResult.degraded_files}</div>
                                    <div className="text-[10px] text-yellow-500 font-black uppercase mt-1">Degraded 3-4/5</div>
                                </div>
                                <div className="bg-red-900/20 border border-red-500/20 rounded-2xl p-6 text-center">
                                    <div className="text-3xl font-black text-red-400">{verificationResult.failed_files}</div>
                                    <div className="text-[10px] text-red-500 font-black uppercase mt-1">At Risk 1-2/5</div>
                                </div>
                                <div className="bg-gray-900/40 border border-gray-700 rounded-2xl p-6 text-center">
                                    <div className="text-3xl font-black text-gray-400">{verificationResult.missing_files}</div>
                                    <div className="text-[10px] text-gray-500 font-black uppercase mt-1">Offline 0/5</div>
                                </div>
                            </div>
                            <div className="space-y-2 max-h-[300px] overflow-y-auto pr-2 scrollbar-thin">
                                {verificationResult.files.map((file, idx) => {
                                    const statusColor = file.status === 'ok'
                                        ? 'text-emerald-400 border-emerald-500/20 bg-emerald-500/5'
                                        : file.status === 'degraded'
                                        ? 'text-yellow-400 border-yellow-500/20 bg-yellow-500/5'
                                        : file.status === 'failed'
                                        ? 'text-red-400 border-red-500/20 bg-red-500/5'
                                        : 'text-gray-500 border-gray-500/20 bg-gray-500/5';
                                    return (
                                        <div key={idx} className="p-3 rounded-xl border bg-gray-900/50 border-gray-700/50 flex items-center justify-between gap-3 group">
                                            <code className="text-[10px] text-gray-500 font-mono group-hover:text-blue-400 transition-colors truncate">{file.root_hash.substring(0, 28)}...</code>
                                            <div className="flex items-center gap-2 shrink-0">
                                                <span className="text-[9px] text-gray-600 font-mono">{file.shards_available}/{file.shards_total}</span>
                                                <div className={`text-[10px] font-black px-2 py-0.5 rounded-full border ${statusColor}`}>
                                                    {file.status.toUpperCase()}
                                                </div>
                                            </div>
                                        </div>
                                    );
                                })}
                            </div>
                        </div>
                    ) : (
                        <div className="text-center py-16">
                            <Shield className="w-16 h-16 text-gray-700 mx-auto mb-6" />
                            <p className="text-gray-400 font-bold uppercase text-xs tracking-widest">Click below to start deep scan</p>
                            <button onClick={onStartVerification} className="mt-8 px-10 py-4 bg-emerald-600 hover:bg-emerald-500 text-white font-black rounded-2xl shadow-2xl transition-all active:scale-95">
                                START VERIFICATION
                            </button>
                        </div>
                    )}
                </div>
                <div className="px-8 py-6 border-t border-gray-700 bg-gray-800/50 flex justify-end">
                    <button onClick={onClose} className="px-8 py-3 bg-gray-700 hover:bg-gray-600 text-gray-200 font-bold rounded-xl transition-all">Close</button>
                </div>
            </div>
        </div>
    );
}
