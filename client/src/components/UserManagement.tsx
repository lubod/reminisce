import { useEffect, useState } from "react";
import { observer } from "mobx-react-lite";
import { useStore } from "../stores/RootStore";
import type { ManagedUser } from "../stores/AuthStore";
import { Users, UserPlus, Trash2, Shield, RefreshCw, Check, X, KeyRound, UserCheck, UserX } from "lucide-react";

export const UserManagement = observer(() => {
    const { authStore } = useStore();
    const [users, setUsers] = useState<ManagedUser[]>([]);
    const [isLoading, setIsLoading] = useState(true);
    const [showAddForm, setShowAddForm] = useState(false);
    const [error, setError] = useState("");

    // Add user form state
    const [newUsername, setNewUsername] = useState("");
    const [newPassword, setNewPassword] = useState("");
    const [newRole, setNewRole] = useState("user");
    const [addError, setAddError] = useState("");
    const [isAdding, setIsAdding] = useState(false);

    // Password reset state
    const [resetUserId, setResetUserId] = useState<string | null>(null);
    const [resetPassword, setResetPassword] = useState("");

    const load = async () => {
        setIsLoading(true);
        setError("");
        try {
            const data = await authStore.listUsers();
            setUsers(data);
        } catch {
            setError("Failed to load users");
        } finally {
            setIsLoading(false);
        }
    };

    useEffect(() => { load(); }, []);

    const handleAdd = async (e: React.FormEvent) => {
        e.preventDefault();
        setAddError("");
        setIsAdding(true);
        const result = await authStore.createUser(newUsername, newPassword, newRole);
        setIsAdding(false);
        if (result.success) {
            setShowAddForm(false);
            setNewUsername(""); setNewPassword(""); setNewRole("user");
            load();
        } else {
            setAddError(result.error || "Failed to create user");
        }
    };

    const handleToggleActive = async (user: ManagedUser) => {
        await authStore.updateUser(user.id, { is_active: !user.is_active });
        load();
    };

    const handleRoleChange = async (user: ManagedUser, role: string) => {
        await authStore.updateUser(user.id, { role });
        load();
    };

    const handleDelete = async (user: ManagedUser) => {
        if (!window.confirm(`Delete user "${user.username}"? This cannot be undone.`)) return;
        await authStore.deleteUser(user.id);
        load();
    };

    const handlePasswordReset = async (userId: string) => {
        if (resetPassword.length < 8) return;
        await authStore.updateUser(userId, { password: resetPassword });
        setResetUserId(null);
        setResetPassword("");
    };

    const roleColor = (role: string) => {
        if (role === "admin") return "text-amber-400 bg-amber-900/20 border-amber-700/40";
        if (role === "viewer") return "text-gray-400 bg-gray-800 border-gray-700";
        return "text-blue-400 bg-blue-900/20 border-blue-700/40";
    };

    return (
        <div className="space-y-6">
            {/* Header */}
            <div className="bg-gray-800/80 rounded-2xl border border-gray-700 overflow-hidden">
                <div className="px-8 py-6 border-b border-gray-700 bg-gray-800/50 flex justify-between items-center">
                    <div>
                        <h2 className="text-2xl font-bold text-gray-100 flex items-center gap-3">
                            <Users className="w-7 h-7 text-blue-400" /> User Management
                        </h2>
                        <p className="text-gray-500 text-sm mt-1">Manage who can access Reminisce</p>
                    </div>
                    <div className="flex gap-3">
                        <button onClick={load} className="p-2.5 bg-gray-700 hover:bg-gray-600 rounded-xl text-gray-300">
                            <RefreshCw className={`w-5 h-5 ${isLoading ? "animate-spin" : ""}`} />
                        </button>
                        <button onClick={() => setShowAddForm(v => !v)} className="flex items-center gap-2 px-5 py-2.5 bg-blue-600 hover:bg-blue-500 text-white font-bold rounded-xl transition-all active:scale-95">
                            <UserPlus className="w-4 h-4" /> Add User
                        </button>
                    </div>
                </div>

                {/* Add user form */}
                {showAddForm && (
                    <div className="px-8 py-6 bg-blue-900/10 border-b border-blue-800/30">
                        <h3 className="text-sm font-bold text-blue-300 uppercase tracking-widest mb-4 flex items-center gap-2">
                            <UserPlus className="w-4 h-4" /> New User
                        </h3>
                        <form onSubmit={handleAdd} className="flex flex-wrap gap-4 items-end">
                            <div className="flex-1 min-w-[160px]">
                                <label className="text-xs text-gray-400 mb-1 block">Username</label>
                                <input value={newUsername} onChange={e => setNewUsername(e.target.value)} required minLength={3}
                                    className="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded-lg text-gray-100 text-sm focus:outline-none focus:border-blue-500"
                                    placeholder="username" />
                            </div>
                            <div className="flex-1 min-w-[160px]">
                                <label className="text-xs text-gray-400 mb-1 block">Password</label>
                                <input type="password" value={newPassword} onChange={e => setNewPassword(e.target.value)} required minLength={8}
                                    className="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded-lg text-gray-100 text-sm focus:outline-none focus:border-blue-500"
                                    placeholder="Min 8 chars" />
                            </div>
                            <div>
                                <label className="text-xs text-gray-400 mb-1 block">Role</label>
                                <select value={newRole} onChange={e => setNewRole(e.target.value)}
                                    className="px-3 py-2 bg-gray-700 border border-gray-600 rounded-lg text-gray-100 text-sm focus:outline-none focus:border-blue-500">
                                    <option value="user">User</option>
                                    <option value="viewer">Viewer</option>
                                    <option value="admin">Admin</option>
                                </select>
                            </div>
                            <div className="flex gap-2">
                                <button type="submit" disabled={isAdding} className="flex items-center gap-2 px-4 py-2 bg-blue-600 hover:bg-blue-500 text-white text-sm font-bold rounded-lg transition-all disabled:opacity-50">
                                    {isAdding ? <RefreshCw className="w-4 h-4 animate-spin" /> : <Check className="w-4 h-4" />} Create
                                </button>
                                <button type="button" onClick={() => { setShowAddForm(false); setAddError(""); }} className="px-4 py-2 bg-gray-700 hover:bg-gray-600 text-gray-300 text-sm rounded-lg transition-all">
                                    <X className="w-4 h-4" />
                                </button>
                            </div>
                        </form>
                        {addError && <p className="text-xs text-red-400 mt-3">{addError}</p>}
                    </div>
                )}

                {/* User list */}
                <div className="p-6">
                    {error && <p className="text-sm text-red-400 mb-4">{error}</p>}
                    {isLoading ? (
                        <div className="flex justify-center py-12"><RefreshCw className="w-8 h-8 animate-spin text-gray-500" /></div>
                    ) : (
                        <div className="space-y-3">
                            {users.map(user => (
                                <div key={user.id} className={`rounded-xl border p-4 flex items-center gap-4 transition-colors ${user.is_active ? "bg-gray-900/50 border-gray-700/60" : "bg-gray-900/20 border-gray-700/30 opacity-60"}`}>
                                    {/* Avatar */}
                                    <div className="w-10 h-10 rounded-full bg-gray-700 flex items-center justify-center text-gray-300 font-bold text-sm flex-shrink-0">
                                        {user.username[0].toUpperCase()}
                                    </div>

                                    {/* Info */}
                                    <div className="flex-1 min-w-0">
                                        <div className="flex items-center gap-2">
                                            <span className="font-bold text-gray-100">{user.username}</span>
                                            <span className={`text-[10px] font-bold px-2 py-0.5 rounded-full border ${roleColor(user.role)}`}>
                                                {user.role.toUpperCase()}
                                            </span>
                                            {!user.is_active && <span className="text-[10px] font-bold px-2 py-0.5 rounded-full text-red-400 bg-red-900/20 border border-red-700/40">DISABLED</span>}
                                        </div>
                                        <div className="text-xs text-gray-500 mt-0.5 flex gap-3">
                                            <span>Joined {new Date(user.created_at).toLocaleDateString()}</span>
                                            {user.last_login_at && <span>Last login {new Date(user.last_login_at).toLocaleDateString()}</span>}
                                        </div>
                                    </div>

                                    {/* Password reset inline */}
                                    {resetUserId === user.id ? (
                                        <div className="flex items-center gap-2">
                                            <input type="password" value={resetPassword} onChange={e => setResetPassword(e.target.value)}
                                                className="px-2 py-1.5 bg-gray-700 border border-gray-600 rounded-lg text-gray-100 text-xs w-32 focus:outline-none focus:border-blue-500"
                                                placeholder="New password" minLength={8} autoFocus />
                                            <button onClick={() => handlePasswordReset(user.id)} disabled={resetPassword.length < 8}
                                                className="p-1.5 bg-blue-600 hover:bg-blue-500 text-white rounded-lg disabled:opacity-40 transition-colors">
                                                <Check className="w-3.5 h-3.5" />
                                            </button>
                                            <button onClick={() => { setResetUserId(null); setResetPassword(""); }}
                                                className="p-1.5 bg-gray-700 hover:bg-gray-600 text-gray-300 rounded-lg transition-colors">
                                                <X className="w-3.5 h-3.5" />
                                            </button>
                                        </div>
                                    ) : (
                                        /* Actions */
                                        <div className="flex items-center gap-2 flex-shrink-0">
                                            {/* Role selector */}
                                            <select value={user.role} onChange={e => handleRoleChange(user, e.target.value)}
                                                className="px-2 py-1.5 bg-gray-700 border border-gray-600 rounded-lg text-gray-300 text-xs focus:outline-none focus:border-blue-500">
                                                <option value="user">User</option>
                                                <option value="viewer">Viewer</option>
                                                <option value="admin">Admin</option>
                                            </select>

                                            {/* Reset password */}
                                            <button onClick={() => setResetUserId(user.id)} title="Reset password"
                                                className="p-2 rounded-lg bg-gray-700 hover:bg-indigo-700 text-gray-400 hover:text-white transition-colors">
                                                <KeyRound className="w-3.5 h-3.5" />
                                            </button>

                                            {/* Toggle active */}
                                            <button onClick={() => handleToggleActive(user)} title={user.is_active ? "Disable user" : "Enable user"}
                                                className={`p-2 rounded-lg transition-colors ${user.is_active ? "bg-gray-700 hover:bg-amber-700 text-gray-400 hover:text-white" : "bg-gray-700 hover:bg-green-700 text-gray-400 hover:text-white"}`}>
                                                {user.is_active ? <UserX className="w-3.5 h-3.5" /> : <UserCheck className="w-3.5 h-3.5" />}
                                            </button>

                                            {/* Delete */}
                                            <button onClick={() => handleDelete(user)} title="Delete user"
                                                className="p-2 rounded-lg bg-gray-700 hover:bg-red-700 text-gray-400 hover:text-white transition-colors">
                                                <Trash2 className="w-3.5 h-3.5" />
                                            </button>
                                        </div>
                                    )}
                                </div>
                            ))}
                        </div>
                    )}
                </div>
            </div>

            {/* Role legend */}
            <div className="bg-gray-800/40 rounded-xl border border-gray-700/50 p-5">
                <h4 className="text-xs font-bold text-gray-500 uppercase tracking-widest mb-3 flex items-center gap-2">
                    <Shield className="w-3.5 h-3.5" /> Role Permissions
                </h4>
                <div className="grid grid-cols-1 sm:grid-cols-3 gap-4 text-sm">
                    <div className="flex items-start gap-3">
                        <span className="text-[10px] font-bold px-2 py-0.5 rounded-full text-amber-400 bg-amber-900/20 border border-amber-700/40 mt-0.5">ADMIN</span>
                        <span className="text-gray-400">Full access — manage users, settings, view all media</span>
                    </div>
                    <div className="flex items-start gap-3">
                        <span className="text-[10px] font-bold px-2 py-0.5 rounded-full text-blue-400 bg-blue-900/20 border border-blue-700/40 mt-0.5">USER</span>
                        <span className="text-gray-400">Upload and manage their own media</span>
                    </div>
                    <div className="flex items-start gap-3">
                        <span className="text-[10px] font-bold px-2 py-0.5 rounded-full text-gray-400 bg-gray-800 border border-gray-700 mt-0.5">VIEWER</span>
                        <span className="text-gray-400">Read-only access, cannot upload</span>
                    </div>
                </div>
            </div>
        </div>
    );
});
