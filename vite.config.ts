import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  build: {
    // macOS WKWebView / Windows WebView2 都支持的保守目标
    target: "safari15",
    minify: "esbuild",
  },
});
