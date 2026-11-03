import { useState } from "react";
import type { FormEvent } from "react";
import { Navigate } from "react-router-dom";

import { useAuth } from "@/auth/AuthContext";

export default function LoginPage(): JSX.Element {
  const { user, login, error } = useAuth();
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [submitting, setSubmitting] = useState(false);

  if (user) {
    return <Navigate to="/tenders" replace />;
  }

  const onSubmit = async (event: FormEvent<HTMLFormElement>): Promise<void> => {
    event.preventDefault();
    setSubmitting(true);
    try {
      await login(username, password);
    } catch {
      // The provider exposes the message; nothing else to do here.
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="flex min-h-screen items-center justify-center bg-slate-100 px-4">
      <div className="card w-full max-w-sm p-8">
        <h1 className="text-xl font-semibold text-slate-900">
          Tender<span className="text-brand-600">Sourcing</span>
        </h1>
        <p className="mt-1 text-sm text-slate-500">
          Подбор поставщиков для тендерных позиций
        </p>

        <form className="mt-6 space-y-4" onSubmit={(event) => void onSubmit(event)}>
          <div>
            <label className="label" htmlFor="username">
              Логин
            </label>
            <input
              id="username"
              className="input"
              autoComplete="username"
              value={username}
              onChange={(event) => setUsername(event.target.value)}
              required
            />
          </div>
          <div>
            <label className="label" htmlFor="password">
              Пароль
            </label>
            <input
              id="password"
              type="password"
              className="input"
              autoComplete="current-password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              required
            />
          </div>
          {error ? (
            <p className="rounded-lg bg-red-50 px-3 py-2 text-sm text-red-700">{error}</p>
          ) : null}
          <button type="submit" className="btn-primary w-full" disabled={submitting}>
            {submitting ? "Вход…" : "Войти"}
          </button>
        </form>

        <p className="mt-6 text-xs text-slate-400">
          Учётная запись создаётся из переменных окружения AUTH_USERNAME / AUTH_PASSWORD.
        </p>
      </div>
    </div>
  );
}
