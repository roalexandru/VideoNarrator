import { defineConfig } from "vite";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig(({ command }) => ({
  plugins: [tailwindcss()],
  base: command === "serve" ? "/" : "/VideoNarrator/",
  build: {
    outDir: "dist",
    emptyOutDir: true,
  },
}));
