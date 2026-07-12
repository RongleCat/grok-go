import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { BrandProvider } from "./components/brand-context";
import { ErrorBoundary } from "./components/error-boundary";
import { ToastProvider } from "./components/ui/toast";
import { I18nProvider } from "./i18n/context";
import "./index.css";

const rootEl = document.getElementById("root");
if (!rootEl) {
  throw new Error("root element not found");
}

ReactDOM.createRoot(rootEl).render(
  <React.StrictMode>
    <ErrorBoundary>
      <I18nProvider>
        <BrandProvider>
          <ToastProvider>
            <App />
          </ToastProvider>
        </BrandProvider>
      </I18nProvider>
    </ErrorBoundary>
  </React.StrictMode>
);
