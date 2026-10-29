import { readdirSync, readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'

// Names and avatars open the quick card and, from it, the full profile, from inside any modal (QOTY
// voting, the people picker, ...). Those must appear on top and usable: the full profile above
// every full-window layer, the quick card above the full profile.
const dirs = ['./', '../routes/']
const files = dirs.flatMap((d) =>
  readdirSync(new URL(d, import.meta.url)).filter((f) => f.endsWith('.svelte')).map((f) => ({ name: d + f, css: readFileSync(new URL(d + f, import.meta.url), 'utf8') })),
)
/** Every `selector { ... }` rule in `css` with its z-index, if any. */
function rules(css: string) {
  return [...css.matchAll(/([^{}\n]+)\{([^{}]*)\}/g)].map(([, sel, body]) => ({ sel: sel.trim(), body, z: Number(/z-index:\s*(\d+)/.exec(body)?.[1] ?? NaN) }))
}
const zOf = (file: string, sel: string) => rules(files.find((f) => f.name === file)!.css).find((r) => r.sel === sel)!.z
// Full-window layers that can't hold a name: the quote image viewer shows one picture.
const NO_NAMES = new Set(['./QuoteItem.svelte .lightbox'])

describe('stacking', () => {
  it('the full profile sits above every full-window layer, and the quick card above it', () => {
    const full = zOf('./FullProfile.svelte', '.overlay')
    const card = zOf('./ProfileCardPopover.svelte', '.pop')
    const layers = files.flatMap((f) =>
      rules(f.css)
        .filter((r) => /position:\s*fixed/.test(r.body) && /inset:\s*0/.test(r.body) && !Number.isNaN(r.z))
        .map((r) => ({ id: `${f.name} ${r.sel}`, z: r.z })),
    ).filter((l) => l.id !== './FullProfile.svelte .overlay' && !NO_NAMES.has(l.id))
    expect(layers.length).toBeGreaterThan(10)
    for (const l of layers) expect(full, l.id).toBeGreaterThan(l.z)
    expect(card).toBeGreaterThan(full)
  })
})
