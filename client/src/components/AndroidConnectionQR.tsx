import { useEffect, useState } from "react";
import { QRCodeSVG } from "qrcode.react";
import { Smartphone, Copy, CheckCircle, RefreshCw, AlertTriangle } from "lucide-react";
import axios from "../api/axiosConfig";

interface ConnectionInfo {
    node_id: string;
    local_ip?: string;
    tunnel_url?: string;
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

    const { protocol, port } = window.location;
    const suffix = port ? `:${port}` : '';
    const urlEntries: { label: string; url: string }[] = [];
    if (connectionInfo?.local_ip)
        urlEntries.push({ label: 'Local Network', url: `${protocol}//${connectionInfo.local_ip}${suffix}` });
    if (connectionInfo?.tunnel_url)
        urlEntries.push({ label: 'Remote (via VPS)', url: connectionInfo.tunnel_url });
    if (urlEntries.length === 0)
        urlEntries.push({ label: 'Server URL', url: window.location.origin });

    const qrData = connectionInfo ? JSON.stringify({
        node_id: connectionInfo.node_id,
        server_urls: urlEntries.map(e => e.url),
    }) : '';

    const handleCopyJson = () => {
        navigator.clipboard.writeText(qrData);
        setCopied(true);
        setTimeout(() => setCopied(false), 2000);
    };

    if (isLoading) {
        return (
            <div className="bg-gray-800 rounded-2xl border border-gray-700 p-16 flex items-center justify-center">
                <RefreshCw className="w-6 h-6 animate-spin text-blue-400 mr-3" />
                <span className="text-gray-400">Loading connection info...</span>
            </div>
        );
    }

    if (error || !connectionInfo) {
        return (
            <div className="bg-gray-800 rounded-2xl border border-gray-700 p-12 flex flex-col items-center justify-center gap-4">
                <AlertTriangle className="w-10 h-10 text-orange-400" />
                <p className="text-orange-300 font-semibold">{error || 'Unable to fetch connection information'}</p>
                <button onClick={fetchConnectionInfo} className="px-4 py-2 bg-orange-600 hover:bg-orange-500 text-white text-sm rounded-lg transition-colors">
                    Retry
                </button>
            </div>
        );
    }

    return (
        <div className="bg-gray-800 rounded-2xl border border-gray-700 overflow-hidden">
            <div className="px-8 py-6 border-b border-gray-700 bg-gray-800/50 flex items-center gap-3">
                <div className="p-2.5 bg-blue-900/40 rounded-xl">
                    <Smartphone className="w-6 h-6 text-blue-400" />
                </div>
                <div>
                    <h2 className="text-xl font-bold text-gray-100">Android App Setup</h2>
                    <p className="text-gray-500 text-sm">Scan to connect your device — no manual config needed</p>
                </div>
            </div>

            <div className="p-8">
                <div className="grid grid-cols-1 md:grid-cols-2 gap-10 items-start">
                    {/* Left: QR code + action buttons */}
                    <div className="flex flex-col items-center gap-5">
                        <div className="bg-white p-5 rounded-2xl shadow-2xl shadow-black/30">
                            <QRCodeSVG value={qrData} size={260} level="M" includeMargin={false} />
                        </div>
                        <p className="text-xs text-gray-500 text-center -mt-1">
                            Scan with the Reminisce Android app
                        </p>
                        <div className="flex gap-3 w-full">
                            <button onClick={handleCopyJson} className="flex-1 flex items-center justify-center gap-2 px-4 py-2.5 bg-gray-700 hover:bg-gray-600 text-gray-300 text-sm font-medium rounded-xl transition-colors">
                                {copied ? <><CheckCircle className="w-4 h-4 text-green-400" /><span className="text-green-400">Copied!</span></> : <><Copy className="w-4 h-4" /><span>Copy JSON</span></>}
                            </button>
                            <button onClick={fetchConnectionInfo} className="flex-1 flex items-center justify-center gap-2 px-4 py-2.5 bg-blue-600 hover:bg-blue-500 text-white text-sm font-medium rounded-xl transition-colors">
                                <RefreshCw className="w-4 h-4" /><span>Refresh</span>
                            </button>
                        </div>
                    </div>

                    {/* Right: instructions + connection details */}
                    <div className="space-y-6">
                        <div className="bg-blue-900/20 border border-blue-500/30 rounded-xl p-6">
                            <h4 className="text-blue-300 font-bold text-sm mb-4 uppercase tracking-wide">Quick Setup</h4>
                            <ol className="space-y-3">
                                {[
                                    'Install the Reminisce app on your Android device',
                                    'Open the app and tap "Scan QR Code" on the login screen',
                                    'Point your camera at this QR code',
                                    'Enter your username and password to sign in',
                                ].map((step, i) => (
                                    <li key={i} className="flex items-start gap-3 text-sm text-blue-200/80">
                                        <span className="flex-shrink-0 w-6 h-6 bg-blue-500/20 border border-blue-500/40 rounded-full flex items-center justify-center text-xs font-bold text-blue-300">{i + 1}</span>
                                        {step}
                                    </li>
                                ))}
                            </ol>
                        </div>

                        <div className="space-y-3">
                            <h4 className="text-xs font-bold text-gray-500 uppercase tracking-widest">Connection Details</h4>
                            <div>
                                <div className="text-xs text-gray-500 mb-1">Node ID</div>
                                <code className="block text-gray-300 font-mono text-[11px] bg-gray-900/60 px-3 py-2 rounded-lg border border-gray-700 break-all">
                                    {connectionInfo.node_id}
                                </code>
                            </div>
                            {urlEntries.map(({ label, url }) => (
                                <div key={label}>
                                    <div className="text-xs text-gray-500 mb-1">{label}</div>
                                    <code className="block text-green-300 font-mono text-[11px] bg-gray-900/60 px-3 py-2 rounded-lg border border-gray-700 break-all">
                                        {url}
                                    </code>
                                </div>
                            ))}
                        </div>
                    </div>
                </div>
            </div>
        </div>
    );
};
