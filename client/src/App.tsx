import type { ReactNode } from "react";
import { BrowserRouter as Router, Routes, Route, Navigate } from "react-router-dom";
import { LoginForm } from "./components/LoginForm";
import { Layout } from "./components/Layout";
import { Dashboard } from "./components/Dashboard";
import { MediaBrowser } from "./components/MediaBrowser";
import { PresentationMode } from "./components/PresentationMode";
import { People } from "./components/People";
import { useStore } from "./stores/RootStore";
import { observer } from "mobx-react-lite";

const ProtectedRoute = observer(({ children }: { children: ReactNode }) => {
  const { authStore } = useStore();
  if (!authStore.isAuthenticated) {
    return <Navigate to="/login" replace />;
  }
  return <>{children}</>;
});

const App = observer(() => {
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
        </Route>
      </Routes>
    </Router>
  );
});

export default App;
