// @vitest-environment jsdom
// The selected server tab and sidebar row, from each component's own compiled CSS (jsdom applies the
// cascade; it doesn't lay out, so "no width shift" is checked as "the same font weight").
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { compile } from 'svelte/compiler'
import { describe, expect, it } from 'vitest'

const loaded = new Set<string>()
function scoped(file: string) {
  const scope = `s-${file.replace(/\W/g, '-')}`
  if (!loaded.has(file)) {
    loaded.add(file)
    const css = compile(readFileSync(resolve(__dirname, file), 'utf8'), { filename: file, css: 'external', cssHash: () => scope }).css!.code
    const style = document.createElement('style')
    style.textContent = css
    document.head.append(style)
  }
  return scope
}
const probe = (html: string) => {
  document.body.innerHTML = html
  return getComputedStyle(document.body.querySelector<HTMLElement>('[data-probe]')!)
}

describe('the selected server tab and sidebar row', () => {
  for (const [file, cls] of [['../routes/Shell.svelte', 'tab'], ['ChannelList.svelte', 'row'], ['ConversationList.svelte', 'row']]) {
    it(`${file}: .${cls}.active is a background and colour, never a bolder weight (that widens it)`, () => {
      const s = scoped(file)
      const plain = probe(`<div><button data-probe class="${cls} ${s}"></button></div>`)
      const weight = plain.fontWeight
      const color = plain.color
      const active = probe(`<div><button data-probe class="${cls} active ${s}"></button></div>`)
      expect(active.fontWeight).toBe(weight)
      expect(active.color).not.toBe(color)
    })
  }

  it('unselected channel and conversation rows share one colour, --text-row', () => {
    for (const file of ['ChannelList.svelte', 'ConversationList.svelte']) {
      const s = scoped(file)
      expect(probe(`<div><button data-probe class="row ${s}"></button></div>`).color, file).toBe('var(--text-row)')
    }
    expect(readFileSync(resolve(__dirname, '../app.css'), 'utf8')).toMatch(/--text-row: #9a9eab;/)
  })
})
