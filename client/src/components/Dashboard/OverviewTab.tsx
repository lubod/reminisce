import type { ReactNode } from "react";
import { Image, Video, Users, CheckCircle, FileText, Brain, HardDrive, Clock } from "lucide-react";
import type { DashboardStats, SystemStats } from "../../stores/StatsStore";

interface OverviewTabProps {
    stats: DashboardStats | null;
    systemStats: SystemStats | null;
}

interface ProgressCardProps {
    total: number;
    pending: number;
    secPerItem: number;
    title: string;
    icon: ReactNode;
    borderClass: string;
    titleClass: string;
    barClass: string;
}

function formatEta(etaSec: number): string {
    if (etaSec < 60) return `~${etaSec}s`;
    if (etaSec < 3600) return `~${Math.round(etaSec / 60)}m`;
    return `~${(etaSec / 3600).toFixed(1)}h`;
}

function AiProgressCard({
    total,
    pending,
    secPerItem,
    title,
    icon,
    borderClass,
    titleClass,
    barClass,
}: ProgressCardProps) {
    if (pending <= 0) return null;
    const done = total - pending;
    const pct = total > 0 ? Math.round((done / total) * 100) : 0;
    const etaStr = formatEta(Math.round(pending * secPerItem));

    return (
        <div className={`bg-gray-800 border ${borderClass} rounded-xl p-5`}>
            <div className="flex items-center justify-between mb-3">
                <div className={`flex items-center gap-2 ${titleClass} font-semibold text-sm`}>
                    {icon}
                    {title}
                </div>
                <span className="text-xs text-gray-400">
                    {pending.toLocaleString()} pending · ETA {etaStr}
                </span>
            </div>
            <div className="w-full bg-gray-700 rounded-full h-2.5 mb-2">
                <div
                    className={`${barClass} h-2.5 rounded-full transition-all duration-500`}
                    style={{ width: `${pct}%` }}
                />
            </div>
            <div className="flex justify-between text-xs text-gray-500">
                <span>{done.toLocaleString()} processed</span>
                <span>{pct}%</span>
                <span>{total.toLocaleString()} total</span>
            </div>
        </div>
    );
}

export function OverviewTab({ stats, systemStats }: OverviewTabProps) {
    const statCards = [
        { title: "Total Images", value: stats?.total_images, icon: <Image className="w-8 h-8 text-blue-500" /> },
        { title: "Total Videos", value: stats?.total_videos, icon: <Video className="w-8 h-8 text-purple-500" /> },
        { title: "Total Users", value: stats?.total_users, icon: <Users className="w-8 h-8 text-green-500" /> },
        { title: "Images with Description", value: stats?.images_with_description, icon: <FileText className="w-8 h-8 text-yellow-500" /> },
        { title: "Images with Embedding", value: stats?.images_with_embedding, icon: <Brain className="w-8 h-8 text-indigo-500" /> },
        { title: "Starred Images", value: stats?.starred_images, icon: <Image className="w-8 h-8 text-red-500" /> },
        { title: "Integrity Verified", value: stats?.verified_images, icon: <CheckCircle className="w-8 h-8 text-pink-500" /> },
        { title: "Total Faces", value: stats?.total_faces, icon: <Users className="w-8 h-8 text-cyan-500" /> },
        { title: "Total People", value: stats?.total_persons, icon: <Users className="w-8 h-8 text-orange-500" /> },
        {
            title: "Disk Available",
            value: systemStats?.disk_available_gb !== undefined ? `${systemStats.disk_available_gb.toFixed(1)} GB` : '...',
            icon: <HardDrive className="w-8 h-8 text-emerald-500" />,
        },
    ];

    return (
        <div className="space-y-6">
            <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-6">
                {statCards.map((card, index) => (
                    <div key={index} className="bg-gray-800 shadow-lg rounded-xl border border-gray-700 p-5 flex items-center transition-transform hover:scale-[1.02]">
                        <div className="p-3 bg-gray-900/50 rounded-lg">{card.icon}</div>
                        <div className="ml-5">
                            <dt className="text-xs font-medium text-gray-500 uppercase tracking-wider">{card.title}</dt>
                            <dd className="text-2xl font-bold text-gray-100">{card.value?.toLocaleString() ?? '...'}</dd>
                        </div>
                    </div>
                ))}
            </div>
            {stats && (
                <>
                    <AiProgressCard
                        total={stats.total_images}
                        pending={stats.images_face_pending}
                        secPerItem={0.63}
                        title="Face Detection In Progress"
                        icon={<Clock className="w-4 h-4" />}
                        borderClass="border-yellow-700/50"
                        titleClass="text-yellow-300"
                        barClass="bg-yellow-500"
                    />
                    <AiProgressCard
                        total={stats.total_images}
                        pending={stats.images_description_pending}
                        secPerItem={12}
                        title="AI Description In Progress"
                        icon={<Brain className="w-4 h-4" />}
                        borderClass="border-blue-700/50"
                        titleClass="text-blue-300"
                        barClass="bg-blue-500"
                    />
                </>
            )}
        </div>
    );
}
