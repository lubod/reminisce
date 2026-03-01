import { useEffect, useState } from "react";
import { QRCodeSVG } from "qrcode.react";
import { Smartphone, Copy, CheckCircle, RefreshCw, AlertTriangle } from "lucide-react";
import axios from "../api/axiosConfig";

interface ConnectionInfo {
    node_id: string;
    relay_url: string;
    listen_addr: string;
}

export const AndroidConnectionQR = () => {
    const [connectionInfo, setConnectionInfo] = useState<ConnectionInfo | null>(null);
    const [isLoading, setIsLoading] = useState(true);
    const [error, setError] = useState<string | null>(null);
    const [copied, setCopied] = useState(false);

    const fetchConnectionInfo = async () => {
        setIsLoading(true);
        setError(null);
        try {
            const response = await axios.get('/p2p/connection');
            setConnectionInfo(response.data);
        } catch (err) {
            console.error('Failed to fetch connection info:', err);
            setError('Failed to load connection info');
        } finally {
            setIsLoading(false);
        }
    };

    useEffect(() => {
        fetchConnectionInfo();
    }, []);

    const handleCopyJson = () => {
        if (connectionInfo) {
            const jsonStr = JSON.stringify(connectionInfo, null, 2);
            navigator.clipboard.writeText(jsonStr);
            setCopied(true);
            setTimeout(() => setCopied(false), 2000);
        }
    };

    if (isLoading) {
        return (
            <div className="bg-gray-700/20 rounded-xl p-6 border border-gray-600/20">
                <div className="flex items-center justify-center">
                    <RefreshCw className="w-5 h-5 animate-spin text-gray-400" />
                    <span className="ml-2 text-gray-400">Loading connection info...</span>
                </div>
            </div>
        );
    }

    if (error || !connectionInfo) {
        return (
            <div className="bg-gray-700/20 rounded-xl p-6 border border-gray-600/20">
                <div className="flex items-start">
                    <AlertTriangle className="w-5 h-5 text-orange-400 mt-0.5 mr-3 flex-shrink-0" />
                    <div>
                        <h4 className="text-orange-300 font-semibold text-sm">Connection Info Unavailable</h4>
                        <p className="text-orange-200/70 text-sm mt-1">{error || 'Unable to fetch connection information'}</p>
                        <button
                            onClick={fetchConnectionInfo}
                            className="mt-3 px-3 py-1.5 bg-orange-600 hover:bg-orange-500 text-white text-xs rounded transition-colors"
                        >
                            Retry
                        </button>
                    </div>
                </div>
            </div>
        );
    }

    // Create QR code data
    const qrData = JSON.stringify({
        node_id: connectionInfo.node_id,
        relay_url: connectionInfo.relay_url,
        listen_addr: connectionInfo.listen_addr,
        server_url: window.location.origin
    });

    return (
        <div className="bg-gray-700/20 rounded-xl p-6 border border-gray-600/20">
            <h3 className="text-sm font-semibold text-gray-300 mb-4 flex items-center uppercase tracking-wide">
                <Smartphone className="w-4 h-4 mr-2 text-blue-400" />
                Android App Setup
            </h3>

            <div className="space-y-4">
                {/* QR Code */}
                <div className="flex flex-col items-center bg-white p-4 rounded-lg">
                    <QRCodeSVG
                        value={qrData}
                        size={200}
                        level="M"
                        includeMargin={true}
                    />
                    <p className="text-xs text-gray-600 mt-2 text-center">
                        Scan with Android app to connect
                    </p>
                </div>

                {/* Instructions */}
                <div className="bg-blue-900/20 border border-blue-500/30 rounded-lg p-3">
                    <h4 className="text-blue-300 font-semibold text-xs mb-2">Quick Setup:</h4>
                    <ol className="text-blue-200/80 text-xs space-y-1 list-decimal list-inside">
                        <li>Open Reminisce Android app</li>
                        <li>Tap "Scan QR Code" on login screen</li>
                        <li>Scan this QR code</li>
                        <li>Enter your username and password</li>
                    </ol>
                </div>

                {/* Connection Details */}
                <div className="space-y-2">
                    <div className="text-xs">
                        <span className="text-gray-500">Node ID:</span>
                        <code className="block text-gray-300 font-mono text-[10px] bg-gray-800/50 px-2 py-1 rounded mt-1 break-all">
                            {connectionInfo.node_id}
                        </code>
                    </div>

                    {connectionInfo.relay_url && (
                        <div className="text-xs">
                            <span className="text-gray-500">Relay URL:</span>
                            <code className="block text-gray-300 font-mono text-[10px] bg-gray-800/50 px-2 py-1 rounded mt-1 break-all">
                                {connectionInfo.relay_url}
                            </code>
                        </div>
                    )}

                    {connectionInfo.listen_addr && (
                        <div className="text-xs">
                            <span className="text-gray-500">Listen Address:</span>
                            <code className="block text-gray-300 font-mono text-[10px] bg-gray-800/50 px-2 py-1 rounded mt-1">
                                {connectionInfo.listen_addr}
                            </code>
                        </div>
                    )}
                </div>

                {/* Copy Button */}
                <button
                    onClick={handleCopyJson}
                    className="w-full flex items-center justify-center gap-2 px-4 py-2 bg-gray-700 hover:bg-gray-600 text-gray-300 text-xs rounded transition-colors"
                >
                    {copied ? (
                        <>
                            <CheckCircle className="w-4 h-4 text-green-400" />
                            <span className="text-green-400">Copied!</span>
                        </>
                    ) : (
                        <>
                            <Copy className="w-4 h-4" />
                            <span>Copy JSON</span>
                        </>
                    )}
                </button>

                {/* Refresh Button */}
                <button
                    onClick={fetchConnectionInfo}
                    className="w-full flex items-center justify-center gap-2 px-4 py-2 bg-blue-600 hover:bg-blue-500 text-white text-xs rounded transition-colors"
                >
                    <RefreshCw className="w-4 h-4" />
                    <span>Refresh Connection Info</span>
                </button>
            </div>
        </div>
    );
};
