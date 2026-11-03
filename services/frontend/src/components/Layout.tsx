import { NavLink, Outlet } from "react-router-dom";

import { useAuth } from "@/auth/AuthContext";

const NAV = [
  { to: "/tenders", label: "Тендеры" },
  { to: "/activity", label: "Журнал" },
  { to: "/usage", label: "Расходы API" },
];

export function Layout(): JSX.Element {
  const { user, logout } = useAuth();

  return (
    <div className="min-h-screen bg-slate-50">
      <header className="border-b border-slate-200 bg-white">
        <div className="mx-auto flex max-w-7xl items-center justify-between gap-6 px-6 py-3">
          <div className="flex items-center gap-8">
            <span className="text-lg font-semibold tracking-tight text-slate-900">
              Tender<span className="text-brand-600">Sourcing</span>
            </span>
            <nav className="flex items-center gap-1">
              {NAV.map((item) => (
                <NavLink
                  key={item.to}
                  to={item.to}
                  className={({ isActive }) =>
                    `rounded-lg px-3 py-1.5 text-sm font-medium transition ${
                      isActive
                        ? "bg-brand-50 text-brand-700"
                        : "text-slate-600 hover:bg-slate-100 hover:text-slate-900"
                    }`
                  }
                >
                  {item.label}
                </NavLink>
              ))}
            </nav>
          </div>
          <div className="flex items-center gap-3 text-sm text-slate-600">
            <span className="hidden sm:inline">{user?.display_name || user?.username}</span>
            <button
              type="button"
              className="btn-secondary"
              onClick={() => {
                void logout();
              }}
            >
              Выйти
            </button>
          </div>
        </div>
      </header>
      <main className="mx-auto max-w-7xl px-6 py-8">
        <Outlet />
      </main>
    </div>
  );
}
