import { Navigate, Outlet } from "react-router-dom";

import { useAuth } from "@/auth/AuthContext";

export function ProtectedRoute(): JSX.Element {
  const { user, loading } = useAuth();

  if (loading) {
    return (
      <div className="flex h-screen items-center justify-center text-sm text-slate-500">
        Проверка сессии…
      </div>
    );
  }

  if (!user) {
    return <Navigate to="/login" replace />;
  }

  return <Outlet />;
}
