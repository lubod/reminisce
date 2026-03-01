import { useState } from "react";
import { observer } from "mobx-react-lite";
import { useStore } from "../stores/RootStore";
import { useNavigate } from "react-router-dom";

export const LoginForm = observer(() => {
    const { authStore } = useStore();
    const [mode, setMode] = useState<'login' | 'register'>('login');
    const [username, setUsername] = useState("");
    const [email, setEmail] = useState("");
    const [password, setPassword] = useState("");
    const [confirmPassword, setConfirmPassword] = useState("");
    const [error, setError] = useState("");
    const [successMessage, setSuccessMessage] = useState("");
    const navigate = useNavigate();

    const clearMessages = () => {
        setError("");
        setSuccessMessage("");
    };

    const handleModeChange = (newMode: 'login' | 'register') => {
        setMode(newMode);
        clearMessages();
    };

    const handleLogin = async () => {
        const result = await authStore.login(username, password);
        if (result.success) {
            navigate("/");
        } else {
            setError(result.error || "Login failed");
        }
    };

    const handleRegister = async () => {
        if (password !== confirmPassword) {
            setError("Passwords do not match");
            return;
        }
        const result = await authStore.register(username, email, password);
        if (result.success) {
            handleModeChange('login');
            setSuccessMessage("Registration successful! Please log in.");
        } else {
            setError(result.error || "Registration failed");
        }
    };

    const handleSubmit = async (e: React.FormEvent) => {
        e.preventDefault();
        clearMessages();
        if (mode === 'login') {
            await handleLogin();
        } else {
            await handleRegister();
        }
    };

    return (
        <div className="flex items-center justify-center min-h-screen bg-gray-900">
            <div className="p-8 bg-gray-800 rounded shadow-md w-96 border border-gray-700">
                <div className="flex border-b border-gray-700 mb-6">
                    <button
                        onClick={() => handleModeChange('login')}
                        className={`w-1/2 py-2 text-center text-lg font-medium transition-colors ${mode === 'login' ? 'text-blue-400 border-b-2 border-blue-400' : 'text-gray-400 hover:text-gray-200'}`}
                    >
                        Login
                    </button>
                    <button
                        onClick={() => handleModeChange('register')}
                        className={`w-1/2 py-2 text-center text-lg font-medium transition-colors ${mode === 'register' ? 'text-blue-400 border-b-2 border-blue-400' : 'text-gray-400 hover:text-gray-200'}`}
                    >
                        Register
                    </button>
                </div>

                <h2 className="mb-6 text-2xl font-bold text-center text-gray-100">{mode === 'login' ? 'Login' : 'Create an Account'}</h2>
                <form onSubmit={handleSubmit}>
                    {successMessage && <p className="mb-4 text-xs italic text-green-500">{successMessage}</p>}
                    {error && <p className="mb-4 text-xs italic text-red-500">{error}</p>}
                    
                    <div className="mb-4">
                        <label className="block mb-2 text-sm font-bold text-gray-300" htmlFor="username">
                            Username
                        </label>
                        <input
                            id="username"
                            type="text"
                            value={username}
                            onChange={(e) => setUsername(e.target.value)}
                            className="w-full px-3 py-2 leading-tight text-gray-100 bg-gray-700 border border-gray-600 rounded shadow appearance-none focus:outline-none focus:border-blue-500 focus:ring-1 focus:ring-blue-500"
                            placeholder="Enter your username"
                            required
                        />
                    </div>

                    {mode === 'register' && (
                        <div className="mb-4">
                            <label className="block mb-2 text-sm font-bold text-gray-300" htmlFor="email">
                                Email
                            </label>
                            <input
                                id="email"
                                type="email"
                                value={email}
                                onChange={(e) => setEmail(e.target.value)}
                                className="w-full px-3 py-2 leading-tight text-gray-100 bg-gray-700 border border-gray-600 rounded shadow appearance-none focus:outline-none focus:border-blue-500 focus:ring-1 focus:ring-blue-500"
                                placeholder="Enter your email"
                                required
                            />
                        </div>
                    )}

                    <div className="mb-4">
                        <label className="block mb-2 text-sm font-bold text-gray-300" htmlFor="password">
                            Password
                        </label>
                        <input
                            id="password"
                            type="password"
                            value={password}
                            onChange={(e) => setPassword(e.target.value)}
                            className="w-full px-3 py-2 leading-tight text-gray-100 bg-gray-700 border border-gray-600 rounded shadow appearance-none focus:outline-none focus:border-blue-500 focus:ring-1 focus:ring-blue-500"
                            placeholder="Enter your password"
                            required
                        />
                    </div>

                    {mode === 'register' && (
                        <div className="mb-4">
                            <label className="block mb-2 text-sm font-bold text-gray-300" htmlFor="confirmPassword">
                                Confirm Password
                            </label>
                            <input
                                id="confirmPassword"
                                type="password"
                                value={confirmPassword}
                                onChange={(e) => setConfirmPassword(e.target.value)}
                                className="w-full px-3 py-2 leading-tight text-gray-100 bg-gray-700 border border-gray-600 rounded shadow appearance-none focus:outline-none focus:border-blue-500 focus:ring-1 focus:ring-blue-500"
                                placeholder="Confirm your password"
                                required
                            />
                        </div>
                    )}

                    <div className="flex items-center justify-between mt-6">
                        <button
                            type="submit"
                            className="px-4 py-2 font-bold text-white bg-blue-600 rounded hover:bg-blue-700 focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-2 focus:ring-offset-gray-800 w-full transition-colors"
                        >
                            {mode === 'login' ? 'Sign In' : 'Sign Up'}
                        </button>
                    </div>
                </form>
            </div>
        </div>
    );
});
