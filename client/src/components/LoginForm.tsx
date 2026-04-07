import { useState, useEffect } from "react";
import { observer } from "mobx-react-lite";
import { useStore } from "../stores/RootStore";
import { useNavigate } from "react-router-dom";
import { Shield, Eye, EyeOff, Loader } from "lucide-react";

export const LoginForm = observer(() => {
    const { authStore } = useStore();
    const [username, setUsername] = useState("");
    const [password, setPassword] = useState("");
    const [confirmPassword, setConfirmPassword] = useState("");
    const [showPassword, setShowPassword] = useState(false);
    const [error, setError] = useState("");
    const [isLoading, setIsLoading] = useState(true);
    const navigate = useNavigate();

    useEffect(() => {
        authStore.checkSetupStatus().finally(() => setIsLoading(false));
    }, [authStore]);

    const handleSetup = async (e: React.FormEvent) => {
        e.preventDefault();
        setError("");
        if (password !== confirmPassword) { setError("Passwords do not match"); return; }
        if (password.length < 8) { setError("Password must be at least 8 characters"); return; }
        setIsLoading(true);
        const result = await authStore.setupAdmin(username, password);
        if (result.success) {
            // Auto-login after setup
            const loginResult = await authStore.login(username, password);
            if (loginResult.success) {
                // Force a full page reload to trigger password save
                window.location.href = "/";
                return;
            }
        }
        setIsLoading(false);
        if (!result.success) setError(result.error || "Setup failed");
    };

    const handleLogin = async (e: React.FormEvent) => {
        e.preventDefault();
        setError("");
        setIsLoading(true);
        const result = await authStore.login(username, password);
        if (result.success) {
            // Force a full page reload to trigger password save
            window.location.href = "/";
            return;
        }
        setIsLoading(false);
        setError(result.error || "Login failed");
    };

    if (isLoading && authStore.needsSetup === undefined) {
        return (
            <div className="flex items-center justify-center min-h-screen bg-gray-900">
                <Loader className="w-8 h-8 text-blue-500 animate-spin" />
            </div>
        );
    }

    // ── First-run setup ──────────────────────────────────────────────────────
    if (authStore.needsSetup) {
        return (
            <div className="flex items-center justify-center min-h-screen bg-gray-900">
                <div className="w-full max-w-sm p-8 bg-gray-800 rounded-2xl shadow-2xl border border-gray-700 relative overflow-hidden">
                    {isLoading && (
                        <div className="absolute inset-0 bg-gray-800/50 backdrop-blur-[1px] flex items-center justify-center z-10">
                            <Loader className="w-8 h-8 text-blue-500 animate-spin" />
                        </div>
                    )}
                    <div className="flex flex-col items-center mb-8">
                        <div className="p-3 bg-blue-900/40 rounded-2xl mb-4">
                            <Shield className="w-10 h-10 text-blue-400" />
                        </div>
                        <h1 className="text-2xl font-bold text-gray-100">Welcome to Reminisce</h1>
                        <p className="text-gray-400 text-sm text-center mt-2">Create your administrator account to get started.</p>
                    </div>

                    <form onSubmit={handleSetup} className="space-y-4">
                        {error && <p className="text-xs text-red-400 bg-red-900/20 border border-red-800 rounded-lg px-3 py-2">{error}</p>}

                        <div>
                            <label htmlFor="setup-username" className="block text-sm font-medium text-gray-300 mb-1.5">Username</label>
                            <input
                                id="setup-username" type="text" name="username" autoComplete="username"
                                value={username} onChange={e => setUsername(e.target.value)}
                                className="w-full px-3 py-2.5 bg-gray-700 border border-gray-600 rounded-lg text-gray-100 focus:outline-none focus:border-blue-500 focus:ring-1 focus:ring-blue-500"
                                placeholder="admin" autoFocus required minLength={3} disabled={isLoading}
                            />
                        </div>

                        <div>
                            <label htmlFor="setup-password" className="block text-sm font-medium text-gray-300 mb-1.5">Password</label>
                            <div className="relative">
                                <input
                                    id="setup-password" type={showPassword ? "text" : "password"} name="password"
                                    autoComplete="new-password"
                                    value={password} onChange={e => setPassword(e.target.value)}
                                    className="w-full px-3 py-2.5 bg-gray-700 border border-gray-600 rounded-lg text-gray-100 focus:outline-none focus:border-blue-500 focus:ring-1 focus:ring-blue-500 pr-10"
                                    placeholder="Min 8 characters" required minLength={8} disabled={isLoading}
                                />
                                <button type="button" onClick={() => setShowPassword(v => !v)} className="absolute right-3 top-1/2 -translate-y-1/2 text-gray-400 hover:text-gray-200" disabled={isLoading}>
                                    {showPassword ? <EyeOff className="w-4 h-4" /> : <Eye className="w-4 h-4" />}
                                </button>
                            </div>
                        </div>

                        <div>
                            <label htmlFor="setup-confirm" className="block text-sm font-medium text-gray-300 mb-1.5">Confirm Password</label>
                            <input
                                id="setup-confirm" type={showPassword ? "text" : "password"} name="confirmPassword"
                                autoComplete="new-password"
                                value={confirmPassword} onChange={e => setConfirmPassword(e.target.value)}
                                className="w-full px-3 py-2.5 bg-gray-700 border border-gray-600 rounded-lg text-gray-100 focus:outline-none focus:border-blue-500 focus:ring-1 focus:ring-blue-500"
                                placeholder="Repeat password" required disabled={isLoading}
                            />
                        </div>

                        <button type="submit" disabled={isLoading} className="w-full py-3 mt-2 bg-blue-600 hover:bg-blue-700 text-white font-bold rounded-xl transition-all active:scale-95 disabled:opacity-50">
                            Create Admin Account
                        </button>
                    </form>
                </div>
            </div>
        );
    }

    // ── Normal login ─────────────────────────────────────────────────────────
    return (
        <div className="flex items-center justify-center min-h-screen bg-gray-900">
            <div className="w-full max-w-sm p-8 bg-gray-800 rounded-2xl shadow-2xl border border-gray-700 relative overflow-hidden">
                {isLoading && (
                    <div className="absolute inset-0 bg-gray-800/50 backdrop-blur-[1px] flex items-center justify-center z-10">
                        <Loader className="w-8 h-8 text-blue-500 animate-spin" />
                    </div>
                )}
                <div className="flex flex-col items-center mb-8">
                    <div className="p-3 bg-blue-900/40 rounded-2xl mb-4">
                        <Shield className="w-10 h-10 text-blue-400" />
                    </div>
                    <h1 className="text-2xl font-bold text-gray-100">Reminisce</h1>
                    <p className="text-gray-400 text-sm mt-1">Sign in to your account</p>
                </div>

                <form onSubmit={handleLogin} className="space-y-4">
                    {error && <p className="text-xs text-red-400 bg-red-900/20 border border-red-800 rounded-lg px-3 py-2">{error}</p>}

                    <div>
                        <label htmlFor="login-username" className="block text-sm font-medium text-gray-300 mb-1.5">Username</label>
                        <input
                            id="login-username" type="text" name="username" autoComplete="username"
                            value={username} onChange={e => setUsername(e.target.value)}
                            className="w-full px-3 py-2.5 bg-gray-700 border border-gray-600 rounded-lg text-gray-100 focus:outline-none focus:border-blue-500 focus:ring-1 focus:ring-blue-500"
                            placeholder="Enter your username" autoFocus required disabled={isLoading}
                        />
                    </div>

                    <div>
                        <label htmlFor="login-password" className="block text-sm font-medium text-gray-300 mb-1.5">Password</label>
                        <div className="relative">
                            <input
                                id="login-password" type={showPassword ? "text" : "password"} name="password"
                                autoComplete="current-password"
                                value={password} onChange={e => setPassword(e.target.value)}
                                className="w-full px-3 py-2.5 bg-gray-700 border border-gray-600 rounded-lg text-gray-100 focus:outline-none focus:border-blue-500 focus:ring-1 focus:ring-blue-500 pr-10"
                                placeholder="Enter your password" required disabled={isLoading}
                            />
                            <button type="button" onClick={() => setShowPassword(v => !v)} className="absolute right-3 top-1/2 -translate-y-1/2 text-gray-400 hover:text-gray-200" disabled={isLoading}>
                                {showPassword ? <EyeOff className="w-4 h-4" /> : <Eye className="w-4 h-4" />}
                            </button>
                        </div>
                    </div>

                    <button type="submit" disabled={isLoading} className="w-full py-3 mt-2 bg-blue-600 hover:bg-blue-700 text-white font-bold rounded-xl transition-all active:scale-95 disabled:opacity-50">
                        Sign In
                    </button>
                </form>
            </div>
        </div>
    );
});
