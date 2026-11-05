// #103: menus, popovers, dialogs and the tooltip flashed as they appeared (WebKitGTK, much worse
// under CPU load). Their entrances animate opacity (and a small scale), which the engine runs on a
// compositing layer it makes when the animation starts and drops when it ends: two repaints, at
// the two moments the element changes look, that a busy machine can miss a frame on. Declaring
// `will-change` gives each one its layer from the start, for as long as it's shown, so nothing is
// promoted or demoted mid-entrance. And the entrances may only animate what the compositor alone
// can: opacity and transform (never layout or paint properties).
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

const css = readFileSync(resolve(__dirname, 'app.css'), 'utf8')

/** The declarations of the first plain rule whose selector list is exactly `selector`. */
function decls(selector: string): string {
  const re = new RegExp(`(?:^|\\n)${selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\s*{([^}]*)}`)
  const m = re.exec(css)
  expect(m, `a rule for ${selector}`).not.toBeNull()
  return m![1]
}

/** The properties a @keyframes block animates. */
function animated(name: string): string[] {
  const m = new RegExp(`@keyframes ${name}\\s*{([\\s\\S]*?)}\\s*}`).exec(css)
  expect(m, `@keyframes ${name}`).not.toBeNull()
  return [...m![1].matchAll(/([a-z-]+)\s*:/g)].map((x) => x[1])
}

describe('entrances stay on the compositor (#103)', () => {
  it('pop-in and fade-in animate only opacity and transform', () => {
    for (const name of ['pop-in', 'fade-in']) {
      for (const prop of animated(name)) expect(['opacity', 'transform'], `${name}: ${prop}`).toContain(prop)
    }
  })

  it('each entrance declares the layer it animates on, so it is never promoted or dropped mid-way', () => {
    expect(decls('.pop-in')).toMatch(/will-change:\s*opacity;/)
    expect(decls('.fade-in')).toMatch(/will-change:\s*opacity;/)
  })

  it('never will-change: transform, which would trap fixed overlays opened inside a dialog', () => {
    // e.g. the crop dialog (position: fixed; inset: 0) is opened from inside the Settings dialog
    expect(css).not.toMatch(/will-change:[^;]*transform/)
  })

  it('the tooltip keeps its layer: showing it is only an opacity change', () => {
    const tip = decls('.pulse-tooltip')
    expect(tip).toMatch(/will-change:\s*opacity/)
    expect(tip).toMatch(/transition:\s*opacity [^,;]*;/) // opacity only
  })
})
