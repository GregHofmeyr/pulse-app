// @vitest-environment jsdom
// The selected server tab and sidebar row, from each component's own compiled CSS (jsdom applies the
// cascade; it doesn't lay out, so "no width shift" is checked as "the same font weight").
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { compile } from 'svelte/compiler'
import { describe, expect, it } from 'vitest'

// the colour tokens live in app.css: computed values resolve var(), so they must be defined
const app = document.createElement('style')
app.textContent = readFileSync(resolve(__dirname, '../app.css'), 'utf8')
document.head.append(app)

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
/** A token's computed colour. */
const token = (name: string) => probe(`<div data-probe style="color: var(${name})"></div>`).color

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
      expect(probe(`<div><button data-probe class="row ${s}"></button></div>`).color, file).toBe(token('--text-row'))
    }
    expect(readFileSync(resolve(__dirname, '../app.css'), 'utf8')).toMatch(/--text-row: #9a9eab;/)
  })
})
