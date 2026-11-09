// Which web engine draws the app, as a class on <html> for app.css. The Linux app is WebKitGTK (no "Chrome" in its
// user agent); Windows is WebView2 (Chromium), and so is a Chromium browser showing the UI in development.

/** The Linux app's engine: WebKit on Linux, not Chromium (whose user agent also says AppleWebKit). */
export const isWebKitGtk = (ua: string): boolean => ua.includes('Linux') && ua.includes('AppleWebKit') && !/Chrom(e|ium)\//.test(ua)

/** `.webkitgtk` on the root on WebKitGTK only (#103: its entrances fade without the scale). */
export function markEngine(root: HTMLElement = document.documentElement, ua: string = navigator.userAgent): void {
  root.classList.toggle('webkitgtk', isWebKitGtk(ua))
}
