// @vitest-environment jsdom
// The password fields keep their form's sizing now that the <input> lives inside PasswordInput (#131):
// a caller's scoped `input { … }` no longer reaches it, so each caller styles it through :global.
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { compile } from 'svelte/compiler'
import { afterEach, describe, expect, it } from 'vitest'

const cssOf = (file: string, scope: string) =>
  compile(readFileSync(resolve(__dirname, file), 'utf8'), { filename: file, css: 'external', cssHash: () => scope }).css!.code

afterEach(() => { document.head.innerHTML = ''; document.body.innerHTML = '' })

/** A caller's label with a PasswordInput in it, both components' CSS loaded. */
const field = (file: string) => {
  const style = document.createElement('style')
  style.textContent = cssOf(file, 'caller') + cssOf('PasswordInput.svelte', 'pw')
  document.head.append(style)
  document.body.innerHTML = '<label class="caller">Password <span class="field pw"><input data-probe class="input pw"><button class="eye pw"></button></span></label>'
  return getComputedStyle(document.querySelector('[data-probe]')!)
}

describe('password fields keep their form sizing', () => {
  it('the sign-in card: 40px tall, like Username', () => {
    expect(field('../routes/Login.svelte').height).toBe('40px')
  })

  it.each(['ChangePassword.svelte', 'TwoFactorSettings.svelte'])('%s: 240px wide, like the Code field', (file) => {
    expect(field(file).width).toBe('240px')
  })
})
