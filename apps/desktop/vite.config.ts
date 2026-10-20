import { readFileSync } from "node:fs";
import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

function previewDelay(): Plugin {
  const gif = Buffer.from("R0lGODlhAQABAAAAACw=", "base64");
  return {
    name: "zl-preview-delay",
    apply: "serve",
    configureServer(server) {
      server.middlewares.use("/mock-library.json", (_req, res) => {
        try {
          res.setHeader("Content-Type", "application/json");
          res.end(readFileSync(new URL("./.dev-data/mock-library.json", import.meta.url)));
        } catch {
          res.statusCode = 404;
          res.end("no sample data in .dev-data/mock-library.json");
        }
      });
      server.middlewares.use("/__delay", (req, res) => {
        const ms = Math.min(20000, Number(new URL(req.url ?? "", "http://x").searchParams.get("ms")) || 0);
        setTimeout(() => {
          res.setHeader("Content-Type", "image/gif");
          res.end(gif);
        }, ms);
      });
    },
  };
}

export default defineConfig({
  plugins: [react(), tailwindcss(), previewDelay()],
  clearScreen: false,
  server: { port: 1430, strictPort: true },
  envPrefix: ["VITE_", "TAURI_"],
  build: { target: "es2022", sourcemap: false },
});
