import react from "@vitejs/plugin-react";
import { internalIpV4 } from "internal-ip";
import { defineConfig, PluginOption } from "vite";
import tsconfigPaths from "vite-tsconfig-paths";
import { watch } from "vite-plugin-watch";
import path from "path";
import topLevelAwait from "vite-plugin-top-level-await";

const mobile = !!/android|ios/.exec(process.env.TAURI_ENV_PLATFORM);

const reactDevTools = (): PluginOption => {
  return {
    name: "react-devtools",
    apply: "serve", // Only apply this plugin during development
    transformIndexHtml(html) {
      return {
        html,
        tags: [
          {
            tag: "script",
            attrs: {
              src: "http://localhost:8097",
            },
            injectTo: "head",
          },
        ],
      };
    },
  };
};

// https://vitejs.dev/config/
export default defineConfig(async () => ({
  plugins: [
    react(),
    tsconfigPaths(),
    topLevelAwait(),
    reactDevTools(),
    // Automatically regenerate the typescript bindings when there's a change to the rust code
    watch({
      pattern: ["../swap/src/**/*"],
      command: "yarn run gen-bindings",
      silent: true,
    }),
    // this makes it so that the former plugin can recognize changes to the swap crate
    {
      name: "watch-swap",
      configureServer(server) {
        server.watcher.add(path.resolve(__dirname, "../swap/src"));
      },
    },
  ],
  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: mobile ? "0.0.0.0" : false,
    hmr: mobile
      ? {
          protocol: "ws",
          host: await internalIpV4(),
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**", "!../swap/**/*"],
    },
  },
}));
