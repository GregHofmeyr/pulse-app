// Every test file starts from a known window focus: jsdom's document.hasFocus() is false unless
// an element has focus, and window focus is one shared value (lib/windowFocus.svelte.ts).
// A test that wants an unfocused window spies on hasFocus or fires `blur` itself.
import { beforeEach } from 'vitest'

if (typeof Document !== 'undefined') {
  Object.defineProperty(Document.prototype, 'hasFocus', { configurable: true, value: () => true })
  // Back to focused before each test (the DOM fallback; Tauri-mode tests drive their own events).
  beforeEach(() => void window.dispatchEvent(new window.Event('focus')))
}
