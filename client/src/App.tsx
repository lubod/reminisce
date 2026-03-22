import type { ReactNode } from "react";
import { useEffect } from "react";
import { BrowserRouter as Router, Routes, Route, Navigate } from "react-router-dom";
import { LoginForm } from "./components/LoginForm";
import { Layout } from "./components/Layout";
import { Dashboard } from "./components/Dashboard";
import { MediaBrowser } from "./components/MediaBrowser";
import { PresentationMode } from "./components/PresentationMode";
import { People } from "./components/People";
import { DuplicatesBrowser } from "./components/DuplicatesBrowser";
import { TrashBrowser } from "./components/TrashBrowser";
import { useStore } from "./stores/RootStore";
import { observer } from "mobx-react-lite";
import { Loader } from "lucide-react";

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
          <Route path="trash" element={<TrashBrowser />} />
        </Route>
      </Routes>
    </Router>
  );
});

export default App;
