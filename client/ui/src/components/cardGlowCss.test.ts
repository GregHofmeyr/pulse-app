import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { compile } from 'svelte/compiler'
import { describe, expect, it } from 'vitest'

// The card's glow (ProfileCard: 22px blur, 4px offset) reaches 26px past it. A scrolling box clips
// shadows at its own edge, so a box the card sits in flush cuts the glow into a sharp-cornered square.
const css = (file: string) => compile(readFileSync(resolve(__dirname, file), 'utf8'), { filename: file, css: 'external' }).css!.code
const rule = (code: string, sel: string) => code.match(new RegExp(`\\.${sel}\\.[\\w-]+\\s*\\{([^}]*)\\}`))?.[1] ?? ''

describe("the card's glow isn't clipped", () => {
  it('the quick card popover leaves room for it (without moving the card)', () => {
    const pop = rule(css('ProfileCardPopover.svelte'), 'pop')
    expect(pop).toMatch(/padding: ?26px/)
    expect(pop).toMatch(/margin: ?-26px/)
    // the room around the card isn't a click target: a press there is outside the card
    expect(pop).toMatch(/pointer-events: ?none/)
  })

  it("the full profile's card column leaves room for it", () => {
    const left = rule(css('FullProfile.svelte'), 'left')
    expect(left).toMatch(/padding: ?26px/)
    expect(left).toMatch(/margin: ?-26px/)
  })
})
