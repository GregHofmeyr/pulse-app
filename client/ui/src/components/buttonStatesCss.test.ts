// @vitest-environment jsdom
// Button looks and states that jsdom can check from each component's own compiled CSS (it applies
// the cascade, including :disabled and :focus; it can't hover, and it drops a `background` shorthand
// that holds a var(), so colours are checked through `color`).
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { compile } from 'svelte/compiler'
import { describe, expect, it } from 'vitest'

const loaded = new Set<string>()
/** Load `file`'s CSS under a scope class named after it, and return that class. */
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

describe('dialog footer buttons', () => {
  it('the crop dialog: Apply is the accent button and Cancel a bordered one, not browser defaults; disabled shows', () => {
    const s = scoped('CropDialog.svelte')
    const apply = (dis = '') => probe(`<div class="actions ${s}"><button data-probe class="primary ${s}" ${dis}></button></div>`)
    expect(apply().color).toBe('var(--on-accent)')
    expect(apply().opacity).not.toBe('0.5')
    expect(apply('disabled').opacity).toBe('0.5')
    const cancel = probe(`<div class="actions ${s}"><button data-probe class="${s}"></button></div>`)
    expect(cancel.color).toBe('var(--text-2)')
    expect(cancel.height).toBe('36px')
  })

  it("a clip's Save is the accent button, and dims while it saves", () => {
    const s = scoped('ClipActions.svelte')
    const save = (dis = '') => probe(`<div><button data-probe class="primary ${s}" ${dis}></button></div>`)
    expect(save().color).toBe('var(--on-accent)')
    expect(save('disabled').opacity).toBe('0.5')
  })

  it('Create server, disabled until it has a name, looks disabled', () => {
    const s = scoped('../routes/Shell.svelte')
    const create = (dis = '') => probe(`<div><button data-probe class="primary ${s}" ${dis}></button></div>`)
    expect(create().opacity).toBe('1')
    expect(create('disabled').opacity).toBe('0.5')
    // the probe matches what the template renders
    expect(readFileSync(resolve(__dirname, '../routes/Shell.svelte'), 'utf8')).toContain('<button class="primary" disabled={!newName.trim()}>Create</button>')
  })

  it('the people picker search shows focus (it turned the focus ring off and had no other sign)', () => {
    const app = document.createElement('style')
    app.textContent = readFileSync(resolve(__dirname, '../app.css'), 'utf8')
    document.head.prepend(app)
    const s = scoped('PeoplePicker.svelte')
    document.body.innerHTML = `<div><input class="search ${s}"></div>`
    const input = document.querySelector<HTMLInputElement>('input')!
    input.focus()
    expect(getComputedStyle(input).outline).toBe('2px solid var(--accent)')
    app.remove()
  })

  it('the voice settings primary looks disabled when it is', () => {
    const s = scoped('VoiceSettings.svelte')
    expect(probe(`<div><button data-probe class="primary ${s}" disabled></button></div>`).opacity).toBe('0.5')
  })
})
