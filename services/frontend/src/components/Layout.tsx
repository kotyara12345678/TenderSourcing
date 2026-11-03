import { NavLink, Outlet } from "react-router-dom";

import { useAuth } from "@/auth/AuthContext";

const NAV = [
  { to: "/assistant", label: "AI-запрос" },
  { to: "/tenders", label: "Тендеры" },
  { to: "/activity", label: "Журнал" },
  { to: "/usage", label: "Расходы API" },
];

export function Layout(): JSX.Element {
  const { user, logout } = useAuth();

  return (
    <div className="min-h-screen">
      <header className="sticky top-0 z-20 border-b border-zinc-200 bg-white/85 backdrop-blur">
        <div className="shell flex h-14 items-center gap-8">
          <NavLink to="/tenders" className="flex items-center gap-2 no-underline">
            <span className="grid h-5 w-5 place-items-center rounded bg-zinc-900 text-[10px] font-semibold text-white">
              TS
            </span>
            <span className="text-[14px] font-semibold tracking-[-0.01em] text-zinc-900">
              TenderSourcing
            </span>
          </NavLink>

          <nav className="flex items-center gap-5">
            {NAV.map((item) => (
              <NavLink
                key={item.to}
                to={item.to}
                className={({ isActive }) =>
                  `no-underline transition-colors ${
                    isActive
                      ? "font-medium text-zinc-900"
                      : "text-zinc-500 hover:text-zinc-900"
                  }`
                }
              >
                {item.label}
              </NavLink>
            ))}
          </nav>

          <div className="ml-auto flex items-center gap-3">
            <span className="hidden text-[12px] text-zinc-500 sm:inline">
              {user?.display_name || user?.username}
            </span>
            <button
              type="button"
              className="btn-ghost btn-sm"
              onClick={() => {
                void logout();
              }}
            >
              Выйти
            </button>
          </div>
        </div>
      </header>

      <main className="shell py-8">
        <Outlet />
      </main>
    </div>
  );
}
