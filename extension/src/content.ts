export function startContent(api: Pick<typeof chrome.runtime, "sendMessage">, page: Pick<Document, "visibilityState" | "addEventListener">, location: Pick<Location, "href">) {
  function report() {
    // A document can outlive an extension reload and lose its runtime context.
    try { void api.sendMessage({ type: "lull.visibility", visible: page.visibilityState === "visible", url: location.href }).catch(() => {}); }
    catch { /* Extension context was invalidated. */ }
  }
  page.addEventListener("visibilitychange", report);
  report();
  return setInterval(() => { if (page.visibilityState === "visible") report(); }, 30_000);
}
if (typeof chrome !== "undefined" && typeof document !== "undefined") startContent(chrome.runtime, document, location);
