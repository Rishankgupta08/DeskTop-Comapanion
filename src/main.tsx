import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
// App.css imports Tailwind layers + Google Fonts
import "./App.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
