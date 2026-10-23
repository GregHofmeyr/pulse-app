/**
 * Where the webview loads a stored image: the core's `pulse-media` scheme, which adds the login
 * and caches on disk. WebView2 (Windows) exposes custom schemes as `http://<scheme>.localhost`.
 */
export function mediaUrl(hash: string, ua: string = navigator.userAgent): string {
  return ua.includes('Windows') ? `http://pulse-media.localhost/${hash}` : `pulse-media://localhost/${hash}`
}
