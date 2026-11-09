// @vitest-environment jsdom
// #103: WebKitGTK (the Linux app) gets its own class on <html>, so app.css can leave out what only it draws badly.
import { describe, expect, it } from 'vitest'
import { isWebKitGtk, markEngine } from './engine'

const LINUX_APP = 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko)'
const WINDOWS_APP = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Safari/537.36 Edg/142.0.0.0'
const LINUX_CHROME = 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Safari/537.36'

describe('the engine class (#103)', () => {
  it('is WebKitGTK only in the Linux app: not WebView2, not Chrome on Linux (the UI in a dev browser)', () => {
    expect(isWebKitGtk(LINUX_APP)).toBe(true)
    expect(isWebKitGtk(WINDOWS_APP)).toBe(false)
    expect(isWebKitGtk(LINUX_CHROME)).toBe(false)
  })

  it('sets .webkitgtk on the root there, and nowhere else', () => {
    const root = document.createElement('html')
    markEngine(root, LINUX_APP)
    expect(root.classList.contains('webkitgtk')).toBe(true)
    markEngine(root, WINDOWS_APP)
    expect(root.classList.contains('webkitgtk')).toBe(false)
  })
})
