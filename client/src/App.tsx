import type { ReactNode } from "react";
import { useEffect } from "react";
import { BrowserRouter as Router, Routes, Route, Navigate } from "react-router-dom";
import { LoginForm } from "./components/LoginForm";
import { Layout } from "./components/Layout";
import { Dashboard } from "./components/Dashboard";
import { MediaBrowser } from "./components/MediaBrowser";
import { PresentationMode } from "./components/PresentationMode";
import { OrientationCheck } from "./components/OrientationCheck";
import { People } from "./components/People";
import { DuplicatesBrowser } from "./components/DuplicatesBrowser";
import { TrashBrowser } from "./components/TrashBrowser";
import { useStore } from "./stores/RootStore";
import { observer } from "mobx-react-lite";
import { Loader, FileQuestion } from "lucide-react";

import { ErrorBoundary } from "./components/ErrorBoundary";

const NotFound = () => (
    <div className="flex flex-col items-center justify-center py-24 text-gray-400 animate-in fade-in duration-500">
        <FileQuestion size={56} className="mb-4 text-gray-600" />
        <h2 className="text-2xl font-bold text-gray-200">Page not found</h2>
        <p className="text-sm text-gray-500 mt-2">
            The page you're looking for doesn't exist or has been moved.
        </p>
    </div>
);

const ProtectedRoute = observer(({ children }: { children: ReactNode }) => {
  const { authStore } = useStore();
  if (!authStore.initialized) return null;
  if (authStore.needsSetup || !authStore.isAuthenticated) {
    return <Navigate to="/login" replace />;
  }
  return <>{children}</>;
});

const App = observer(() => {
  const { authStore } = useStore();

  useEffect(() => {
    authStore.initialize();
  }, [authStore]);

  if (!authStore.initialized) {
    return (
      <div className="flex items-center justify-center min-h-screen bg-gray-900">
        <Loader className="w-8 h-8 text-blue-500 animate-spin" />
      </div>
    );
  }

  return (
    <ErrorBoundary>
      <Router>
        <Routes>
          <Route path="/login" element={<LoginForm />} />
          <Route
            path="/"
            element={
              <ProtectedRoute>
                <Layout />
              </ProtectedRoute>
            }
          >
            <Route index element={<Dashboard />} />
            <Route path="media" element={<MediaBrowser />} />
            <Route path="people" element={<People />} />
            <Route path="people/:personId" element={<People />} />
            <Route path="present" element={<PresentationMode />} />
            <Route path="duplicates" element={<DuplicatesBrowser />} />
            <Route path="orientation" element={<OrientationCheck />} />
            <Route path="trash" element={<TrashBrowser />} />
            <Route path="*" element={<NotFound />} />
          </Route>
        </Routes>
      </Router>
    </ErrorBoundary>
  );
});

export default App;
