// @vitest-environment jsdom
// Button looks and states that jsdom can check from app.css's shared classes and each component's own
// compiled CSS (it applies the cascade, including :disabled and :focus; it can't hover, and it drops
// a `background` shorthand that holds a var(), so colours are checked through `color`).
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { compile } from 'svelte/compiler'
import { describe, expect, it } from 'vitest'

// the shared .btn classes live in app.css
const app = document.createElement('style')
app.textContent = readFileSync(resolve(__dirname, '../app.css'), 'utf8')
document.head.append(app)

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

const source = (file: string) => readFileSync(resolve(__dirname, file), 'utf8')

describe('dialog footer buttons', () => {
  it('the crop dialog: Apply is the accent button and Cancel a bordered one, not browser defaults; disabled shows', () => {
    const s = scoped('CropDialog.svelte')
    const apply = (dis = '') => probe(`<div class="actions ${s}"><button data-probe class="btn btn-lg btn-primary ${s}" ${dis}></button></div>`)
    expect(apply().color).toBe('var(--on-accent)')
    expect(apply().opacity).not.toBe('0.5')
    expect(apply('disabled').opacity).toBe('0.5')
    const cancel = probe(`<div class="actions ${s}"><button data-probe class="btn btn-lg btn-secondary ${s}"></button></div>`)
    expect(cancel.color).toBe('var(--text-2)')
    expect(cancel.height).toBe('36px')
    // the probes match what the template renders
    expect(source('CropDialog.svelte')).toContain('<button type="button" class="btn btn-lg btn-secondary" onclick={onCancel}>Cancel</button>')
    expect(source('CropDialog.svelte')).toContain('<button type="button" class="btn btn-lg btn-primary" disabled={!crop}')
  })

  it("a clip's Save is the accent button, and dims while it saves", () => {
    const s = scoped('ClipActions.svelte')
    const save = (dis = '') => probe(`<div><button data-probe class="btn btn-lg btn-primary ${s}" ${dis}></button></div>`)
    expect(save().color).toBe('var(--on-accent)')
    expect(save('disabled').opacity).toBe('0.5')
    expect(source('ClipActions.svelte')).toContain('<button type="button" class="btn btn-lg btn-primary" disabled={busy} onclick={() => void save()}>Save</button>')
  })

  it('Create server, disabled until it has a name, looks disabled', () => {
    const s = scoped('../routes/Shell.svelte')
    const create = (dis = '') => probe(`<div><button data-probe class="btn btn-lg btn-primary ${s}" ${dis}></button></div>`)
    expect(create().opacity).toBe('1')
    expect(create('disabled').opacity).toBe('0.5')
    // the probe matches what the template renders
    expect(source('../routes/Shell.svelte')).toContain('<button class="btn btn-lg btn-primary" disabled={!newName.trim()}>Create</button>')
  })

  it('the people picker search shows focus (it turned the focus ring off and had no other sign)', () => {
    const s = scoped('PeoplePicker.svelte')
    document.body.innerHTML = `<div><input class="search ${s}"></div>`
    const input = document.querySelector<HTMLInputElement>('input')!
    input.focus()
    expect(getComputedStyle(input).outline).toBe('2px solid var(--accent)')
  })

  it('the voice settings primary looks disabled when it is', () => {
    const s = scoped('VoiceSettings.svelte')
    expect(probe(`<div><button data-probe class="btn btn-lg btn-primary ${s}" disabled></button></div>`).opacity).toBe('0.5')
    expect(source('VoiceSettings.svelte')).toContain('<button class="btn btn-lg btn-primary" onclick={toggleTest}>')
  })

  it('a button that is only aria-disabled (so its tooltip still shows) looks disabled too', () => {
    expect(probe('<div><button data-probe class="btn btn-primary" aria-disabled="true"></button></div>').opacity).toBe('0.5')
  })

  it('live and stream actions are the accent, not green (owner pick 5)', () => {
    for (const f of ['GoLiveDialog.svelte', 'ClipEditor.svelte', 'StreamTile.svelte', 'LivePopover.svelte', 'StreamToast.svelte', 'VoicePanel.svelte']) {
      expect(source(f), f).not.toMatch(/background: var\(--ok\)/)
    }
  })
})

describe('text fields', () => {
  it('every field shows the same focus ring: none turns it off or keeps an always-on accent border', () => {
    const fields: [string, string][] = [
      ['ChannelList.svelte', 'input input-sm new'],
      ['LineMenu.svelte', 'input input-sm find'],
      ['TextChannel.svelte', 'input input-sm rename'],
      ['PeoplePicker.svelte', 'input search'],
      ['EmojiPicker.svelte', 'search'],
    ]
    for (const [file, cls] of fields) {
      const s = scoped(file)
      document.body.innerHTML = `<div><input class="${cls} ${s}"></div>`
      const input = document.querySelector<HTMLInputElement>('input')!
      expect(getComputedStyle(input).borderColor, file).not.toBe('var(--accent)')
      input.focus()
      expect(getComputedStyle(input).outline, file).toBe('2px solid var(--accent)')
      // the probe matches what the template renders
      expect(source(file), file).toContain(`class="${cls}"`)
      // jsdom drops a border shorthand that holds a var(), so the accent border is checked in the source
      const own = cls.split(' ').at(-1)
      expect(source(file), file).not.toMatch(new RegExp(`\\.${own}(:focus)? \\{[^}]*(var\\(--accent\\)|outline: none)`))
    }
  })
})
