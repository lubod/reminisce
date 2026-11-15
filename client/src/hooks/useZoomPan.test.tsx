import { describe, it, expect, vi } from "vitest";
import { renderHook, act } from "@testing-library/react";
import { useRef, useState } from "react";
import { useZoomPan, type UseZoomPanOptions, type ZoomPanState } from "./useZoomPan";

function setupHook(options: UseZoomPanOptions = {}, enabled = true) {
    const div = document.createElement("div");
    div.getBoundingClientRect = () =>
        ({ left: 0, top: 0, right: 800, bottom: 600, width: 800, height: 600, x: 0, y: 0, toJSON: () => ({}) }) as DOMRect;
    document.body.appendChild(div);

    const utils = renderHook(
        ({ opts, active }: { opts: UseZoomPanOptions; active: boolean }) => {
            const ref = useRef<HTMLElement | null>(div);
            return useZoomPan([ref], active, opts);
        },
        { initialProps: { opts: options, active: enabled } }
    );

    return { div, ...utils };
}

describe("useZoomPan", () => {
    it("zoomIn, zoomOut, reset, and scale clamping work as expected", () => {
        const { result, div } = setupHook({ minScale: 1, maxScale: 2, buttonStepFactor: 1.5 });
        expect(result.current.scale).toBe(1);

        act(() => result.current.zoomIn());
        expect(result.current.scale).toBeCloseTo(1.5);

        act(() => result.current.zoomIn());
        expect(result.current.scale).toBe(2);

        act(() => result.current.zoomOut());
        expect(result.current.scale).toBeCloseTo(2 / 1.5);

        act(() => result.current.zoomOut());
        act(() => result.current.zoomOut());
        expect(result.current.scale).toBe(1);

        act(() => result.current.zoomIn());
        act(() => result.current.reset());
        expect(result.current.scale).toBe(1);
        expect(result.current.x).toBe(0);
        expect(result.current.y).toBe(0);

        div.remove();
    });

    it("handles native wheel zoom with and without zoomAtCursor and ignores when disabled", () => {
        const { result, rerender, div } = setupHook({ wheelInFactor: 2, zoomAtCursor: true }, true);

        act(() => {
            div.dispatchEvent(
                new WheelEvent("wheel", {
                    deltaY: -100,
                    clientX: 200,
                    clientY: 150,
                    bubbles: true,
                    cancelable: true,
                })
            );
        });

        expect(result.current.scale).toBe(2);
        // cx - (cx - 0) * 2 = -cx, cy - (cy - 0) * 2 = -cy
        expect(result.current.x).toBe(-200);
        expect(result.current.y).toBe(-150);

        act(() => {
            div.dispatchEvent(
                new WheelEvent("wheel", {
                    deltaY: 100,
                    clientX: 200,
                    clientY: 150,
                    bubbles: true,
                    cancelable: true,
                })
            );
        });

        expect(result.current.scale).toBe(1);
        expect(result.current.x).toBe(0);
        expect(result.current.y).toBe(0);

        // With zoomAtCursor disabled, x/y remain unchanged
        rerender({ opts: { wheelInFactor: 2, zoomAtCursor: false }, active: true });
        act(() => {
            div.dispatchEvent(
                new WheelEvent("wheel", {
                    deltaY: -100,
                    clientX: 200,
                    clientY: 150,
                    bubbles: true,
                    cancelable: true,
                })
            );
        });
        expect(result.current.scale).toBe(2);
        expect(result.current.x).toBe(0);
        expect(result.current.y).toBe(0);

        // When disabled, wheel listener is removed and does not change scale
        rerender({ opts: { wheelInFactor: 2, zoomAtCursor: false }, active: false });
        act(() => {
            div.dispatchEvent(
                new WheelEvent("wheel", {
                    deltaY: -100,
                    bubbles: true,
                    cancelable: true,
                })
            );
        });
        expect(result.current.scale).toBe(2);

        div.remove();
    });

    it("handles mouse drag panning, onlyPanWhenZoomed, and non-left clicks", () => {
        const { result, div } = setupHook({ onlyPanWhenZoomed: true, buttonStepFactor: 2 });

        // Non-left mouse button ignored
        act(() => {
            result.current.onMouseDown({ button: 1, clientX: 10, clientY: 10 } as React.MouseEvent);
        });
        expect(result.current.isDragging).toBe(false);

        // At scale=1 with onlyPanWhenZoomed=true, left click does not start drag
        act(() => {
            result.current.onMouseDown({ button: 0, clientX: 10, clientY: 10 } as React.MouseEvent);
            result.current.onMouseMove({ clientX: 50, clientY: 30 } as React.MouseEvent);
        });
        expect(result.current.isDragging).toBe(false);
        expect(result.current.x).toBe(0);
        expect(result.current.y).toBe(0);

        // Zoom in first, then drag pans x/y until mouseup
        act(() => result.current.zoomIn());
        act(() => {
            result.current.onMouseDown({ button: 0, clientX: 10, clientY: 10 } as React.MouseEvent);
        });
        expect(result.current.isDragging).toBe(true);

        act(() => {
            result.current.onMouseMove({ clientX: 55, clientY: 40 } as React.MouseEvent);
        });
        expect(result.current.x).toBe(45);
        expect(result.current.y).toBe(30);

        act(() => result.current.onMouseUp());
        expect(result.current.isDragging).toBe(false);

        act(() => {
            result.current.onMouseMove({ clientX: 100, clientY: 100 } as React.MouseEvent);
        });
        expect(result.current.x).toBe(45);

        div.remove();
    });

    it("supports controlled mode via onUpdate", () => {
        const div = document.createElement("div");
        div.getBoundingClientRect = () =>
            ({ left: 0, top: 0, right: 400, bottom: 300, width: 400, height: 300, x: 0, y: 0, toJSON: () => ({}) }) as DOMRect;
        document.body.appendChild(div);

        const updates: ZoomPanState[] = [];
        const onUpdateSpy = vi.fn((next: ZoomPanState) => updates.push(next));

        const { result } = renderHook(() => {
            const ref = useRef<HTMLElement | null>(div);
            const [state, setState] = useState<ZoomPanState>({ scale: 1, x: 0, y: 0 });
            return useZoomPan([ref], true, {
                buttonStepFactor: 2,
                controlled: {
                    ...state,
                    onUpdate: (next) => {
                        onUpdateSpy(next);
                        setState(next);
                    },
                },
            });
        });

        act(() => result.current.zoomIn());
        expect(onUpdateSpy).toHaveBeenCalledWith({ scale: 2, x: 0, y: 0 });
        expect(result.current.scale).toBe(2);

        act(() => result.current.reset());
        expect(onUpdateSpy).toHaveBeenLastCalledWith({ scale: 1, x: 0, y: 0 });
        expect(result.current.scale).toBe(1);

        div.remove();
    });
});
