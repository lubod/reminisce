import { useEffect, useMemo, useRef, useState, useCallback } from "react";
import { MapContainer, TileLayer, Marker, Popup, useMap, useMapEvents } from "react-leaflet";
import L from "leaflet";
import Supercluster from "supercluster";
import "leaflet/dist/leaflet.css";
import { observer } from "mobx-react-lite";
import { useStore } from "../stores/RootStore";
import type { MapPoint } from "../stores/MediaStore";

interface PointProps {
    hash: string;
    lon: number;
    lat: number;
    place?: string | null;
    created_at: string;
    starred: boolean;
    has_thumbnail: boolean;
}

interface MapFeature {
    id?: string | number;
    geometry: { coordinates: [number, number] };
    properties: PointProps & {
        cluster?: boolean;
        cluster_id?: number;
        point_count?: number;
    };
}

function clusterIcon(count: number): L.DivIcon {
    return L.divIcon({
        className: "",
        html: `<div style="width:40px;height:40px;border-radius:50%;background:#7c3aed;color:#fff;display:flex;align-items:center;justify-content:center;font-size:13px;font-weight:700;border:2px solid #fff;box-shadow:0 1px 4px rgba(0,0,0,.45)">${count}</div>`,
        iconSize: [40, 40],
        iconAnchor: [20, 20],
    });
}

function pointIcon(p: PointProps): L.DivIcon {
    const inner = p.has_thumbnail
        ? `<img src="/api/thumbnail/${p.hash}" style="width:36px;height:36px;border-radius:9px;object-fit:cover;border:2px solid #fff;box-shadow:0 1px 4px rgba(0,0,0,.5)" onerror="this.style.visibility='hidden'"/>`
        : `<div style="width:14px;height:14px;border-radius:50%;background:#2563eb;border:2px solid #fff;box-shadow:0 1px 4px rgba(0,0,0,.5)"></div>`;
    return L.divIcon({
        className: "",
        html: inner,
        iconSize: [36, 36],
        iconAnchor: [18, 18],
    });
}

function ClusterLayer({ points, onOpen }: { points: PointProps[]; onOpen: (hash: string) => void }) {
    const map = useMap();
    const fitted = useRef(false);

    const index = useMemo(() => {
        const sc = new Supercluster({ radius: 60, maxZoom: 16, minZoom: 0, extent: 256 });
        sc.load(
            points.map((p) => ({
                type: "Feature" as const,
                geometry: { type: "Point" as const, coordinates: [p.lon, p.lat] as [number, number] },
                properties: p,
            })),
        );
        return sc;
    }, [points]);

    const [features, setFeatures] = useState<MapFeature[]>([]);

    const recompute = useCallback(() => {
        const b = map.getBounds();
        const zoom = map.getZoom();
        setFeatures(index.getClusters([b.getWest(), b.getSouth(), b.getEast(), b.getNorth()], zoom) as unknown as MapFeature[]);
    }, [map, index]);

    useMapEvents({
        moveend: recompute,
        zoomend: recompute,
    });

    useEffect(() => {
        if (points.length > 0 && !fitted.current) {
            fitted.current = true;
            const b = L.latLngBounds(points.map((p) => [p.lat, p.lon] as L.LatLngTuple));
            if (b.isValid()) map.fitBounds(b, { padding: [40, 40], maxZoom: 14 });
        }
        recompute();
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [points, map]);

    return (
        <>
            {features.map((f) => {
                const [lon, lat] = f.geometry.coordinates;
                if (f.properties.cluster) {
                    const id = f.properties.cluster_id ?? (typeof f.id === "number" ? f.id : 0);
                    const count: number = f.properties.point_count ?? 0;
                    return (
                        <Marker
                            key={`c${f.id}`}
                            position={[lat, lon]}
                            icon={clusterIcon(count)}
                            eventHandlers={{
                                click: () => {
                                    const zoom = index.getClusterExpansionZoom(id);
                                    map.flyTo([lat, lon], zoom);
                                },
                            }}
                        />
                    );
                }
                const p = f.properties as PointProps;
                return (
                    <Marker key={p.hash} position={[lat, lon]} icon={pointIcon(p)}>
                        <Popup>
                            <div className="text-center min-w-36">
                                {p.has_thumbnail && (
                                    <img
                                        src={`/api/thumbnail/${p.hash}`}
                                        alt=""
                                        className="w-40 h-40 object-cover rounded-md mb-2"
                                        onError={(e) => { (e.target as HTMLImageElement).style.display = "none"; }}
                                    />
                                )}
                                <div className="text-xs font-bold text-gray-800">
                                    {new Date(p.created_at).toLocaleDateString()}
                                </div>
                                {p.place && <div className="text-xs text-gray-600 mt-0.5">{p.place}</div>}
                                <button
                                    onClick={() => onOpen(p.hash)}
                                    className="mt-2 px-3 py-1 bg-blue-600 hover:bg-blue-700 text-white text-xs rounded"
                                >
                                    Open
                                </button>
                            </div>
                        </Popup>
                    </Marker>
                );
            })}
        </>
    );
}

export const MapView = observer(() => {
    const { mediaStore } = useStore();

    const points: PointProps[] = (mediaStore.mapPoints ?? []).map((p: MapPoint) => ({
        hash: p.hash,
        lon: p.lon,
        lat: p.lat,
        place: p.place,
        created_at: p.created_at,
        starred: p.starred,
        has_thumbnail: p.has_thumbnail,
    }));

    const openPhoto = (hash: string) => mediaStore.openMapPhoto(hash);

    return (
        <div className="relative h-[70vh] w-full rounded-xl overflow-hidden border border-gray-700">
            {mediaStore.isMapLoading && mediaStore.mapPoints.length === 0 && (
                <div className="absolute inset-0 z-[1000] flex items-center justify-center bg-gray-900/50 text-white text-sm">
                    Loading map…
                </div>
            )}
            {mediaStore.mapError && (
                <div className="absolute top-2 left-1/2 -translate-x-1/2 z-[1000] bg-red-600 text-white text-xs px-3 py-1 rounded shadow">
                    {mediaStore.mapError}
                </div>
            )}
            {!mediaStore.isMapLoading && mediaStore.mapPoints.length === 0 && (
                <div className="absolute inset-0 z-[1000] flex items-center justify-center text-gray-400 text-sm">
                    No geotagged media match the current filters
                </div>
            )}
            <MapContainer center={[49.2, 16]} zoom={5} className="h-full w-full" scrollWheelZoom>
                <TileLayer
                    attribution='&copy; <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a> contributors'
                    url="https://tile.openstreetmap.org/{z}/{x}/{y}.png"
                />
                {points.length > 0 && <ClusterLayer points={points} onOpen={openPhoto} />}
            </MapContainer>
        </div>
    );
});
