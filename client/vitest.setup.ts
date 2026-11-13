interface MemoryStorage {
    readonly length: number;
    clear(): void;
    getItem(key: string): string | null;
    key(index: number): string | null;
    removeItem(key: string): void;
    setItem(key: string, value: string): void;
}

type GlobalWithStorage = typeof globalThis & {
    localStorage?: MemoryStorage;
    window?: { localStorage?: MemoryStorage };
};

const createMemoryStorage = (): MemoryStorage => {
    const store = new Map<string, string>();
    return {
        get length() {
            return store.size;
        },
        clear() {
            store.clear();
        },
        getItem(key: string) {
            return store.get(String(key)) ?? null;
        },
        key(index: number) {
            return Array.from(store.keys())[index] ?? null;
        },
        removeItem(key: string) {
            store.delete(String(key));
        },
        setItem(key: string, value: string) {
            store.set(String(key), String(value));
        },
    };
};

// Node 25+ exposes an accessor on globalThis.localStorage that warns and returns
// undefined without --localstorage-file, which can leave bare `localStorage` undefined
// in Vitest jsdom workers.
const g = globalThis as GlobalWithStorage;
if (typeof g.localStorage === "undefined") {
    const storage =
        typeof g.window !== "undefined" && typeof g.window.localStorage !== "undefined"
            ? g.window.localStorage
            : createMemoryStorage();
    Object.defineProperty(globalThis, "localStorage", {
        value: storage,
        configurable: true,
        writable: true,
    });
    if (typeof g.window !== "undefined" && typeof g.window.localStorage === "undefined") {
        Object.defineProperty(g.window, "localStorage", {
            value: storage,
            configurable: true,
            writable: true,
        });
    }
}
