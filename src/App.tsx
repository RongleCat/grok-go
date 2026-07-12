import { BrowserRouter, Navigate, Route, Routes } from "react-router-dom";
import { AppLayout } from "@/components/layout";
import { OverviewPage } from "@/pages/Overview";
import { AccountsPage } from "@/pages/Accounts";
import { MappingPage } from "@/pages/Mapping";
import { IntegrationsPage } from "@/pages/Integrations";
import { UsagePage } from "@/pages/Usage";
import { LogsPage } from "@/pages/Logs";
import { SettingsPage } from "@/pages/Settings";

export default function App() {
  return (
    <BrowserRouter>
      <Routes>
        <Route element={<AppLayout />}>
          <Route index element={<OverviewPage />} />
          <Route path="accounts" element={<AccountsPage />} />
          <Route path="mapping" element={<MappingPage />} />
          <Route path="integrations" element={<IntegrationsPage />} />
          <Route path="usage" element={<UsagePage />} />
          <Route path="logs" element={<LogsPage />} />
          <Route path="settings" element={<SettingsPage />} />
          <Route path="*" element={<Navigate to="/" replace />} />
        </Route>
      </Routes>
    </BrowserRouter>
  );
}
