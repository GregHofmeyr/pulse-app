// @vitest-environment node
// #139: the "no sound from your mic" notice is a lightly tinted warning block, its text in the warn colour, no side bar.
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { compile } from 'svelte/compiler'
import { describe, expect, it } from 'vitest'

const css = compile(readFileSync(resolve(__dirname, 'VoiceStatus.svelte'), 'utf8'), { filename: 'VoiceStatus.svelte', css: 'external', cssHash: () => 's' }).css!.code
const rule = (sel: string) => css.match(new RegExp(`(?:^|\\n|\\})\\s*${sel.replace(/\./g, '\\.')}\\.s\\s*\\{([^}]*)\\}`))?.[1] ?? ''

describe('mic warning', () => {
  it('tinted block with warn-coloured text and no left bar', () => {
    const m = rule('.vc-mic')
    expect(m).toMatch(/background:\s*var\(--warn-soft\)/)
    expect(m).toMatch(/color:\s*var\(--warn-text\)/)
    expect(m).not.toMatch(/border-left/)
  })
})
