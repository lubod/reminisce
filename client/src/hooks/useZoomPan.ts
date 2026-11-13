import { useCallback, useEffect, useRef, useState, type MouseEvent, type RefObject } from "react";

export interface ZoomPanState {
    scale: number;
    x: number;
    y: number;
}

export interface ControlledZoomPan extends ZoomPanState {
    onUpdate: (next: ZoomPanState) => void;
}

export interface UseZoomPanOptions {
    minScale?: number;
    maxScale?: number;
    wheelInFactor?: number;
    buttonStepFactor?: number;
    onlyPanWhenZoomed?: boolean;
    zoomAtCursor?: boolean;
    controlled?: ControlledZoomPan;
}

const DEFAULT_STATE: ZoomPanState = { scale: 1, x: 0, y: 0 };

/**
 * Shared wheel-zoom and mouse drag-to-pan controller for fullscreen media lightboxes.
 * Supports either internal React state or a controlled external store (e.g. MobX).
 */
export function useZoomPan(
    containerRefs: RefObject<HTMLElement | null>[],
    enabled: boolean,
    options: UseZoomPanOptions = {}
) {
    const {
        minScale = 1,
        maxScale = 10,
        wheelInFactor = 1.15,
        buttonStepFactor = 1.25,
        onlyPanWhenZoomed = false,
        zoomAtCursor = false,
        controlled,
    } = options;

    const [internalState, setInternalState] = useState<ZoomPanState>(DEFAULT_STATE);
    const [isDragging, setIsDragging] = useState(false);

    const state = controlled
        ? { scale: controlled.scale, x: controlled.x, y: controlled.y }
        : internalState;

    const latestRef = useRef({
        state,
        onUpdate: controlled?.onUpdate,
        minScale,
        maxScale,
        wheelInFactor,
        onlyPanWhenZoomed,
        zoomAtCursor,
    });
    latestRef.current = {
        state,
        onUpdate: controlled?.onUpdate,
        minScale,
        maxScale,
        wheelInFactor,
        onlyPanWhenZoomed,
        zoomAtCursor,
    };

    const dragOrigin = useRef({ clientX: 0, clientY: 0, startX: 0, startY: 0 });

    const commit = useCallback((updater: (prev: ZoomPanState) => ZoomPanState) => {
        const currentOnUpdate = latestRef.current.onUpdate;
        if (currentOnUpdate) {
            currentOnUpdate(updater(latestRef.current.state));
        } else {
            setInternalState(updater);
        }
    }, []);

    const clampScale = useCallback(
        (nextScale: number) => Math.min(Math.max(nextScale, minScale), maxScale),
        [minScale, maxScale]
    );

    const zoomIn = useCallback(() => {
        commit((prev) => ({
            ...prev,
            scale: Math.min(prev.scale * buttonStepFactor, maxScale),
        }));
    }, [buttonStepFactor, commit, maxScale]);

    const zoomOut = useCallback(() => {
        commit((prev) => ({
            ...prev,
            scale: Math.max(prev.scale / buttonStepFactor, minScale),
        }));
    }, [buttonStepFactor, commit, minScale]);

    const reset = useCallback(() => {
        commit(() => DEFAULT_STATE);
    }, [commit]);

    // Attach non-passive native wheel listeners so preventDefault suppresses page scroll.
    const refCount = containerRefs.length;
    useEffect(() => {
        if (!enabled) return;
        const cleanups: (() => void)[] = [];

        for (const ref of containerRefs) {
            const el = ref.current;
            if (!el) continue;
            const handleWheel = (e: WheelEvent) => {
                e.preventDefault();
                const cfg = latestRef.current;
                const factor = e.deltaY < 0 ? cfg.wheelInFactor : 1 / cfg.wheelInFactor;
                const rect = el.getBoundingClientRect();
                const cx = e.clientX - rect.left;
                const cy = e.clientY - rect.top;

                commit((prev) => {
                    const nextScale = Math.min(Math.max(prev.scale * factor, cfg.minScale), cfg.maxScale);
                    if (!cfg.zoomAtCursor) {
                        return { ...prev, scale: nextScale };
                    }
                    const ratio = nextScale / prev.scale;
                    return {
                        scale: nextScale,
                        x: cx - (cx - prev.x) * ratio,
                        y: cy - (cy - prev.y) * ratio,
                    };
                });
            };
            el.addEventListener("wheel", handleWheel, { passive: false });
            cleanups.push(() => el.removeEventListener("wheel", handleWheel));
        }

        return () => {
            for (const cleanup of cleanups) cleanup();
        };
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [enabled, refCount, commit]);

    const onMouseDown = useCallback((e: MouseEvent) => {
        if (e.button !== 0) return;
        const cfg = latestRef.current;
        if (cfg.onlyPanWhenZoomed && cfg.state.scale <= 1) return;
        setIsDragging(true);
        dragOrigin.current = {
            clientX: e.clientX,
            clientY: e.clientY,
            startX: cfg.state.x,
            startY: cfg.state.y,
        };
    }, []);

    const onMouseMove = useCallback(
        (e: MouseEvent) => {
            if (!isDragging) return;
            const dx = e.clientX - dragOrigin.current.clientX;
            const dy = e.clientY - dragOrigin.current.clientY;
            commit((prev) => ({
                ...prev,
                x: dragOrigin.current.startX + dx,
                y: dragOrigin.current.startY + dy,
            }));
        },
        [commit, isDragging]
    );

    const onMouseUp = useCallback(() => {
        setIsDragging(false);
    }, []);

    return {
        scale: clampScale(state.scale),
        x: state.x,
        y: state.y,
        isDragging,
        zoomIn,
        zoomOut,
        reset,
        onMouseDown,
        onMouseMove,
        onMouseUp,
    };
}
