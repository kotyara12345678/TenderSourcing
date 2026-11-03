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
    <div className="flex min-h-screen flex-col">
      <div className="shell flex h-14 items-center">
        <span className="flex items-center gap-2">
          <span className="grid h-5 w-5 place-items-center rounded bg-zinc-900 text-[10px] font-semibold text-white">
            TS
          </span>
          <span className="text-[14px] font-semibold tracking-[-0.01em]">TenderSourcing</span>
        </span>
      </div>

      <div className="flex flex-1 items-start justify-center px-5 pt-[12vh]">
        <div className="w-full max-w-[320px]">
          <h1 className="text-[19px] font-semibold tracking-[-0.01em]">Вход в систему</h1>
          <p className="mt-1 text-[13px] text-zinc-500">
            Подбор поставщиков для тендерных позиций
          </p>

          <form className="mt-8 space-y-4" onSubmit={(event) => void onSubmit(event)}>
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
            {error ? <p className="banner-error">{error}</p> : null}
            <button type="submit" className="btn-primary w-full" disabled={submitting}>
              {submitting ? "Вход…" : "Войти"}
            </button>
          </form>

          <p className="mt-8 text-[12px] leading-relaxed text-zinc-400">
            Учётная запись оператора задаётся переменными окружения AUTH_USERNAME и AUTH_PASSWORD.
          </p>
        </div>
      </div>
    </div>
  );
}
