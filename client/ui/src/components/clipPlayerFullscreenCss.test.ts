import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { compile } from 'svelte/compiler'
import { describe, expect, it } from 'vitest'

// Measured in WebKitGTK: in full screen the screen kept the clip's aspect height (a flex item doesn't
// shrink below its content), pushing the control bar off the bottom; and the `aspect-ratio: auto
// !important` override, once lifted on the way out, left the screen 70vh tall with black below.
const code = compile(readFileSync(resolve(__dirname, 'ClipPlayer.svelte'), 'utf8'), { filename: 'ClipPlayer.svelte', css: 'external' }).css!.code
const fullscreenScreen = code.match(/\.clip\.[\w-]+:fullscreen \.screen:where\(\.[\w-]+\)\s*\{([^}]*)\}/)?.[1] ?? ''

describe('the clip player in full screen', () => {
  it('lets the screen shrink so the control bar stays on screen', () => {
    expect(fullscreenScreen).not.toBe('')
    expect(fullscreenScreen).toMatch(/flex: ?1/)
    expect(fullscreenScreen).toMatch(/min-height: ?0/)
  })

  it("doesn't override the aspect ratio, which WebKit doesn't recompute when full screen ends", () => {
    expect(fullscreenScreen).not.toMatch(/aspect-ratio/)
  })
})
