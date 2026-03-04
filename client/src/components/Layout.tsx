import { useState } from "react";
import { observer } from "mobx-react-lite";
import { useStore } from "../stores/RootStore";
import { Outlet, Link, useNavigate, useLocation } from "react-router-dom";
import { LogOut, Activity, Menu, X, RefreshCw, MonitorPlay, Users, Folder, Copy, Trash2 } from "lucide-react";

export const Layout = observer(() => {
    const { authStore, statsStore, mediaStore, uiStore, personStore, duplicatesStore, trashStore } = useStore();
    const navigate = useNavigate();
    const location = useLocation();
    const [isMobileMenuOpen, setMobileMenuOpen] = useState(false);

    const handleLogout = () => {
        authStore.logout();
        navigate("/login");
    };

    // Determine page title and icon based on route
    const getPageInfo = () => {
        if (location.pathname === '/') return { title: 'Dashboard', icon: <Activity className="w-5 h-5" /> };
        if (location.pathname.startsWith('/media')) return { title: 'Media', icon: <Folder className="w-5 h-5" /> };
        if (location.pathname.startsWith('/people')) return { title: 'People', icon: <Users className="w-5 h-5" /> };
        if (location.pathname.startsWith('/present')) return { title: 'Presentation Mode', icon: <MonitorPlay className="w-5 h-5" /> };
        if (location.pathname.startsWith('/duplicates')) return { title: 'Duplicates', icon: <Copy className="w-5 h-5" /> };
        if (location.pathname.startsWith('/trash')) return { title: 'Trash', icon: <Trash2 className="w-5 h-5" /> };
        return { title: 'Reminisce', icon: null };
    };

    // Determine refresh action based on route
    const handleRefresh = () => {
        if (location.pathname === '/') {
            statsStore.fetchAllStats();
        } else if (location.pathname.startsWith('/media')) {
            mediaStore.fetchAllMedia();
        } else if (location.pathname.startsWith('/people')) {
            personStore.fetchPersons();
        } else if (location.pathname.startsWith('/duplicates')) {
            duplicatesStore.fetchDuplicates();
        } else if (location.pathname.startsWith('/trash')) {
            trashStore.fetchTrash();
        }
    };

    // Check if refresh is loading
    const isRefreshing = () => {
        if (location.pathname === '/') return statsStore.isLoading;
        if (location.pathname.startsWith('/media')) return mediaStore.isLoadingMoreAllMedia || mediaStore.isSearching;
        if (location.pathname.startsWith('/people')) return personStore.isLoading;
        if (location.pathname.startsWith('/duplicates')) return duplicatesStore.isLoading;
        if (location.pathname.startsWith('/trash')) return trashStore.isLoading;
        return false;
    };

    const pageInfo = getPageInfo();

    const queryParams = new URLSearchParams(location.search);
    const hideMenu = queryParams.get('hidemenu') === 'true';

    const getLinkClass = (path: string) => {
        const isActive = path === '/' ? location.pathname === '/' : location.pathname.startsWith(path);
        return `inline-flex items-center px-1 pt-1 border-b-2 text-sm font-medium transition-colors duration-200 ${
            isActive 
            ? 'border-blue-500 text-gray-100' 
            : 'border-transparent text-gray-400 hover:border-gray-500 hover:text-gray-200'
        }`;
    };

    const getMobileLinkClass = (path: string) => {
        const isActive = path === '/' ? location.pathname === '/' : location.pathname.startsWith(path);
        return `block pl-3 pr-4 py-2 border-l-4 text-base font-medium transition-colors duration-200 ${
            isActive
            ? 'bg-gray-700 border-blue-500 text-gray-100'
            : 'border-transparent text-gray-400 hover:bg-gray-700 hover:border-gray-500 hover:text-gray-200'
        }`;
    };

    if (!authStore.isAuthenticated) {
        return null; // Or redirect to login, handled by router protection usually
    }

    return (
        <div className="min-h-screen bg-gray-900">
            {!uiStore.isFullscreen && !hideMenu && (
                <nav className="bg-gray-800 border-b border-gray-700 shadow-lg">
                    <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8">
                        <div className="flex justify-between h-16">
                            <div className="flex items-center gap-3">
                                <div className="flex-shrink-0 flex items-center gap-2 text-gray-100 mr-4">
                                    <div className="p-1.5 bg-blue-600 rounded-lg">
                                        <Folder className="w-5 h-5 text-white" />
                                    </div>
                                    <span className="text-xl font-bold tracking-tight">Reminisce</span>
                                </div>
                                <button
                                    onClick={handleRefresh}
                                    disabled={isRefreshing()}
                                    className="p-2 rounded-md text-blue-400 hover:bg-gray-700 disabled:text-gray-600 disabled:cursor-not-allowed transition-colors"
                                    title={`Refresh ${pageInfo.title.toLowerCase()}`}
                                >
                                    <RefreshCw size={18} className={isRefreshing() ? 'animate-spin' : ''} />
                                </button>
                                <div className="hidden sm:ml-6 sm:flex sm:space-x-8">
                                    <Link to="/" className={getLinkClass('/')}>
                                        Dashboard
                                    </Link>
                                    <Link to="/media" className={getLinkClass('/media')}>
                                        Media
                                    </Link>
                                    <Link to="/people" className={getLinkClass('/people')}>
                                        People
                                    </Link>
                                    <Link to="/present" className={getLinkClass('/present')}>
                                        Present
                                    </Link>
                                    <Link to="/duplicates" className={getLinkClass('/duplicates')}>
                                        Duplicates
                                    </Link>
                                    <Link to="/trash" className={getLinkClass('/trash')}>
                                        Trash
                                    </Link>
                                </div>
                            </div>
                            <div className="flex items-center">
                                <div className="sm:hidden">
                                    <button
                                        onClick={() => setMobileMenuOpen(!isMobileMenuOpen)}
                                        className="p-2 rounded-md text-gray-400 hover:text-gray-200 focus:outline-none"
                                    >
                                        {isMobileMenuOpen ? <X className="w-6 h-6" /> : <Menu className="w-6 h-6" />}
                                    </button>
                                </div>
                                <button
                                    onClick={handleLogout}
                                    className="hidden sm:block p-2 rounded-md text-gray-400 hover:text-gray-200 focus:outline-none hover:bg-gray-700 transition-colors"
                                    title="Logout"
                                >
                                    <LogOut className="w-5 h-5" />
                                </button>
                            </div>
                        </div>
                    </div>
                    {isMobileMenuOpen && (
                        <div className="sm:hidden border-t border-gray-700">
                            <div className="pt-2 pb-3 space-y-1">
                                <Link
                                    to="/"
                                    onClick={() => setMobileMenuOpen(false)}
                                    className={getMobileLinkClass('/')}
                                >
                                    Dashboard
                                </Link>
                                <Link
                                    to="/media"
                                    onClick={() => setMobileMenuOpen(false)}
                                    className={getMobileLinkClass('/media')}
                                >
                                    Media
                                </Link>
                                <Link
                                    to="/people"
                                    onClick={() => setMobileMenuOpen(false)}
                                    className={getMobileLinkClass('/people')}
                                >
                                    People
                                </Link>
                                <Link
                                    to="/present"
                                    onClick={() => setMobileMenuOpen(false)}
                                    className={getMobileLinkClass('/present')}
                                >
                                    Present
                                </Link>
                                <Link
                                    to="/duplicates"
                                    onClick={() => setMobileMenuOpen(false)}
                                    className={getMobileLinkClass('/duplicates')}
                                >
                                    Duplicates
                                </Link>
                                <Link
                                    to="/trash"
                                    onClick={() => setMobileMenuOpen(false)}
                                    className={getMobileLinkClass('/trash')}
                                >
                                    Trash
                                </Link>
                                <button
                                    onClick={() => {
                                        setMobileMenuOpen(false);
                                        handleLogout();
                                    }}
                                    className="block w-full text-left pl-3 pr-4 py-2 border-l-4 border-transparent text-base font-medium text-gray-400 hover:bg-gray-700 hover:border-gray-500 hover:text-gray-200"
                                >
                                    Logout
                                </button>
                            </div>
                        </div>
                    )}
                </nav>
            )}

            <main className={`${uiStore.isFullscreen ? 'max-w-none p-0' : 'max-w-7xl mx-auto py-6 px-4 sm:px-6 lg:px-8'}`}>
                <Outlet />
            </main>
        </div>
    );
});
