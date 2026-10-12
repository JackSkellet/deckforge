import { defineConfig } from "vite";
import { viteSingleFile } from "vite-plugin-singlefile";
export default defineConfig({
  base: "./",
  plugins: [viteSingleFile()],
  build: { target: "es2022", outDir: "dist" },
  server: { strictPort: true },
});
