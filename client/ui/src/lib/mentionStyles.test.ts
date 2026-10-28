// @vitest-environment jsdom
import { describe, expect, it } from 'vitest'
import { mentionLookup, styleMentions } from './mentionStyles'
import { emptyState } from './state'

const pill = (name: string) => `<span class="mention">@${name}</span>`
const css: Record<string, string> = {
  U2: 'font-family: Fredoka; --name-scale: 1; color: #ffc4a3',
  U3: '--name-scale: 1; background-image: linear-gradient(90deg, #ffbe7a, #ff7fae); -webkit-background-clip: text; background-clip: text; color: transparent',
}
const ids: Record<string, string> = { sam: 'U2', kim: 'U3' }

describe('styleMentions', () => {
  it('styles known names and leaves unknown pills alone', () => {
    const root = document.createElement('div')
    root.innerHTML = `<p>hi ${pill('sam')} and ${pill('kim')} and ${pill('jo')}</p>`
    styleMentions(root, (n) => ids[n.toLowerCase()] ?? null, (id) => css[id])
    const [sam, kim, jo] = root.querySelectorAll<HTMLElement>('.mention')
    expect(sam.style.fontFamily).toBe('Fredoka')
    expect(sam.style.color).toBe('rgb(255, 196, 163)')
    expect(sam.parentElement!.classList.contains('mention-wrap')).toBe(false)
    // a gradient clips to the text, so the pill background moves to a wrapper
    expect(kim.style.backgroundImage).toContain('linear-gradient')
    expect(kim.parentElement!.classList.contains('mention-wrap')).toBe(true)
    expect(jo.getAttribute('style')).toBeNull()
    expect(jo.parentElement!.tagName).toBe('P')
    // running again (a profile changed) neither double-wraps nor keeps a stale wrapper
    css.U3 = 'color: #a3d8ff'
    styleMentions(root, (n) => ids[n.toLowerCase()] ?? null, (id) => css[id])
    expect(root.querySelectorAll('.mention-wrap').length).toBe(0)
    expect(kim.style.color).toBe('rgb(163, 216, 255)')
    expect(kim.style.backgroundImage).toBe('')
  })
  it('looks names up case-insensitively among people and members', () => {
    const u = (id: string, username: string) => ({ id, username, avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } })
    const s = { ...emptyState(), people: { U2: { user: u('U2', 'sam'), online: true, last_seen_at: null } }, members: { S1: [{ user: u('U3', 'kim'), nickname: null }] } } as never
    const look = mentionLookup(s)
    expect(look('Sam')).toBe('U2')
    expect(look('kim')).toBe('U3')
    expect(look('jo')).toBeNull()
  })
})
