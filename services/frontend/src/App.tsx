import { Navigate, Route, Routes } from "react-router-dom";

import { Layout } from "@/components/Layout";
import { ProtectedRoute } from "@/components/ProtectedRoute";
import ActivityPage from "@/pages/ActivityPage";
import AssistantPage from "@/pages/AssistantPage";
import LoginPage from "@/pages/LoginPage";
import TenderPage from "@/pages/TenderPage";
import TendersPage from "@/pages/TendersPage";
import UsagePage from "@/pages/UsagePage";

export default function App(): JSX.Element {
  return (
    <Routes>
      <Route path="/login" element={<LoginPage />} />
      <Route element={<ProtectedRoute />}>
        <Route element={<Layout />}>
          <Route path="/" element={<Navigate to="/tenders" replace />} />
          <Route path="/assistant" element={<AssistantPage />} />
          <Route path="/tenders" element={<TendersPage />} />
          <Route path="/tenders/:tenderId" element={<TenderPage />} />
          <Route path="/activity" element={<ActivityPage />} />
          <Route path="/usage" element={<UsagePage />} />
          <Route path="*" element={<Navigate to="/tenders" replace />} />
        </Route>
      </Route>
    </Routes>
  );
}
