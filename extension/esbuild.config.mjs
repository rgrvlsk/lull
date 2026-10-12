import { build } from "esbuild";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";

// ponytail: Chromium only for v1; add the Firefox event-page manifest/build in WP3.
const outdir = "dist/chromium";
const { version } = JSON.parse(readFileSync("package.json", "utf8"));
const manifest = JSON.parse(readFileSync("manifests/chromium.json", "utf8"));
await build({ entryPoints: ["src/background.ts"], bundle: true, format: "esm", target: "chrome120", outdir, logLevel: "info" });
// Static content scripts cannot import ES modules.
await build({ entryPoints: ["src/content.ts"], bundle: true, format: "iife", target: "chrome120", outdir, logLevel: "info" });
mkdirSync(outdir, { recursive: true });
writeFileSync(`${outdir}/manifest.json`, `${JSON.stringify({ ...manifest, version }, null, 2)}\n`);
