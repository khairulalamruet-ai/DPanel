import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./index.css";
import "@builtpanel/i18n";
import { initMocks } from "@builtpanel/api";

async function bootstrap() {
  // If backend endpoints do not exist yet, mock via MSW in dev
  if (import.meta.env.DEV) {
    await initMocks();
  }

  const rootElement = document.getElementById("root");
  if (!rootElement) throw new Error("Root element not found");

  ReactDOM.createRoot(rootElement).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>
  );
}

bootstrap();
