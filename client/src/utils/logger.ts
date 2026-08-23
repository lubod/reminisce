const isDev = import.meta.env.DEV;

export const logger = {
    info: (...args: unknown[]) => {
        if (isDev) {
            console.info(...args);
        }
    },
    warn: (...args: unknown[]) => {
        if (isDev) {
            console.warn(...args);
        }
    },
    error: (...args: unknown[]) => {
        // Errors are always reported so production crashes (e.g. ErrorBoundary
        // fallbacks) are visible in the browser console for bug reports.
        console.error(...args);
    },
};

export default logger;
