import { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";

import { ApiError, setCsrfToken } from "@/api/client";
import { login as loginRequest, logout as logoutRequest, me } from "@/api/endpoints";
import type { UserView } from "@/api/types";

interface AuthContextValue {
  user: UserView | null;
  loading: boolean;
  error: string | null;
  login: (username: string, password: string) => Promise<void>;
  logout: () => Promise<void>;
}

const AuthContext = createContext<AuthContextValue | null>(null);

export function AuthProvider({ children }: { children: ReactNode }): JSX.Element {
  const [user, setUser] = useState<UserView | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    me()
      .then((current) => {
        if (cancelled) return;
        setUser(current);
        setCsrfToken(current.csrf_token);
      })
      .catch((requestError: unknown) => {
        if (cancelled) return;
        setUser(null);
        setCsrfToken(null);
        if (!(requestError instanceof ApiError && requestError.status === 401)) {
          setError("Не удалось проверить сессию. Проверьте, что API доступен.");
        }
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const login = useCallback(async (username: string, password: string) => {
    setError(null);
    try {
      const response = await loginRequest(username, password);
      setCsrfToken(response.user.csrf_token);
      setUser(response.user);
    } catch (requestError) {
      const message =
        requestError instanceof ApiError && requestError.status === 401
          ? "Неверный логин или пароль"
          : "Не удалось выполнить вход";
      setError(message);
      throw requestError;
    }
  }, []);

  const logout = useCallback(async () => {
    try {
      await logoutRequest();
    } finally {
      setCsrfToken(null);
      setUser(null);
    }
  }, []);

  const value = useMemo<AuthContextValue>(
    () => ({ user, loading, error, login, logout }),
    [user, loading, error, login, logout],
  );

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
}

export function useAuth(): AuthContextValue {
  const context = useContext(AuthContext);
  if (!context) {
    throw new Error("useAuth must be used inside AuthProvider");
  }
  return context;
}
