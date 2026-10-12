// Builds the shared MV3 source once per browser flavour. Entry points arrive with WP2.2;
// until then the build is a no-op that still validates the configuration.
import { build } from "esbuild";
import { readdirSync, existsSync } from "node:fs";

const flavours = ["chromium", "firefox"];
const entryPoints = existsSync("src")
  ? readdirSync("src").filter((f) => /^(background|content)\.ts$/.test(f)).map((f) => `src/${f}`)
  : [];

for (const flavour of flavours) {
  if (entryPoints.length === 0) {
    console.log(`[${flavour}] no entry points yet, skipping`);
    continue;
  }
  await build({
    entryPoints,
    bundle: true,
    format: "esm",
    target: ["chrome120", "firefox139"],
    outdir: `dist/${flavour}`,
    sourcemap: true,
    define: { __FLAVOUR__: JSON.stringify(flavour) },
    logLevel: "info",
  });
}
