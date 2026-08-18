import react from "@vitejs/plugin-react";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { defineConfig } from "vite";

const sourceRoot = decodeURIComponent(new URL(".", import.meta.url).pathname).replace(
  /^\/([A-Za-z]:)/,
  "$1",
);
const cargoToml = readFileSync(resolve(sourceRoot, "../../Cargo.toml"), "utf8");
const productVersion = cargoToml.match(
  /\[workspace\.package\][\s\S]*?^version\s*=\s*"([^"]+)"/m,
)?.[1];

if (!productVersion) {
  throw new Error("Could not read [workspace.package] version from Cargo.toml");
}

export default defineConfig({
  clearScreen: false,
  plugins: [react()],
  define: {
    __SPEAKEASY_PRODUCT_VERSION__: JSON.stringify(`v${productVersion}`),
  },
  root: sourceRoot,
  server: {
    port: 1420,
    strictPort: true,
  },
});
