import { describe, expect, it } from 'vitest'
import { ZALGO_MARKS, charCount, draftFrom, editsSince, isDirty, isZalgo, nameProblem, normalBio, previewCard, previewUser, toRequest, type Draft } from './profileDraft'
import type { Profile } from './protocol/Profile'
import type { User } from './protocol/User'

const alex: User = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: 'a1', avatar_still: 'a1s', style: { font: 'fredoka', colors: ['#ffbe7a', '#ff7fae'] } }
const profile: Profile = {
  user: alex,
  card: { banner_hash: 'b1', banner_still: null, theme_top: '#c2457a', theme_bottom: '#3a1f5e', pronouns: 'they/them', bio: 'hi\nthere' },
  stats: { voice_seconds: 0, member_since: '2026-10-03T00:00:00Z', top_reaction: null, times_renamed: 0 },
  badges: [],
  name_archive: [],
  showcase: [],
  showcase_chosen: false,
}

describe('profileDraft', () => {
  it('normalBio matches the server: endings, blank leading lines, trimmed end', () => {
    expect(normalBio('a\r\nb\rc')).toBe('a\nb\nc')
    expect(normalBio('\n  indented  \n\n')).toBe('  indented')
    expect(normalBio('\n \n\r\n\n\n  a\nb\nc\nd\ne')).toBe('  a\nb\nc\nd\ne')
    expect(normalBio('  \n\t')).toBe('')
    expect(normalBio('one')).toBe('one')
  })

  it('charCount counts code points', () => {
    expect(charCount('😀'.repeat(40))).toBe(40)
    expect('😀'.repeat(40).length).toBe(80)
  })

  it('editsSince lists only the changed fields', () => {
    const a = draftFrom(profile)
    const b = { ...a, colors: [...a.colors], bio: 'new' }
    expect(editsSince(a, b)).toEqual({ bio: 'new' })
    b.colors[0] = '#000000'
    expect(editsSince(a, b)).toEqual({ bio: 'new', colors: ['#000000', '#ff7fae'] })
    expect(editsSince(a, a)).toEqual({})
  })

  it('round trip draft → request', () => {
    const d = draftFrom(profile)
    expect(d).toEqual({
      avatarHash: 'a1', avatarStill: 'a1s', bannerHash: 'b1', bannerStill: null,
      themeTop: '#c2457a', themeBottom: '#3a1f5e', font: 'fredoka', colors: ['#ffbe7a', '#ff7fae'],
      pronouns: 'they/them', bio: 'hi\nthere', displayName: profile.user.display_name, glow: true,
    })
    expect(toRequest(d)).toEqual({
      avatar_hash: 'a1', banner_hash: 'b1', theme_top: '#c2457a', theme_bottom: '#3a1f5e',
      style: { font: 'fredoka', colors: ['#ffbe7a', '#ff7fae'] }, pronouns: 'they/them', bio: 'hi\nthere',
      display_name: profile.user.display_name, glow: true,
    })
    // the draft holds its own colours: editing it never touches the profile
    d.colors.push('#000000')
    expect(profile.user.style.colors).toHaveLength(2)
  })

  it('the card glow (#156): read from the card, on when an older server sends none, sent and previewed as set', () => {
    expect(draftFrom({ ...profile, card: { ...profile.card, glow: false } }).glow).toBe(false)
    expect(draftFrom({ ...profile, card: { ...profile.card, glow: true } }).glow).toBe(true)
    const d = { ...draftFrom(profile), glow: false }
    expect(toRequest(d).glow).toBe(false)
    expect(previewCard(d).glow).toBe(false)
  })

  it('name rules match the server (profiles/names.rs)', () => {
    expect(nameProblem('Alex 🎸')).toBeNull()
    expect(nameProblem('a\u200db')).toBeNull()
    for (const bad of ['', '   ', 'x'.repeat(33)]) expect(nameProblem(bad), bad).toContain('1 to 32')
    expect(nameProblem('é'.repeat(32))).toBeNull()
    for (const bad of ['a\nb', 'a\tb', 'a\u2028b', 'a\u202eb', 'a\u2066b']) expect(nameProblem(bad), bad).toContain('line breaks')
    for (const bad of ['\u200b', '\u200b\ufeff', '\u3164']) expect(nameProblem(bad), bad).toContain('see')
  })

  it('the display name: trimmed when sent, the username when an older server sent none (#47)', () => {
    const d = { ...draftFrom(profile), displayName: '  Alex 🎸  ' }
    expect(toRequest(d).display_name).toBe('Alex 🎸')
    expect(previewUser(profile.user, d).display_name).toBe('Alex 🎸')
    expect(draftFrom({ ...profile, user: { ...profile.user, display_name: '' } }).displayName).toBe(profile.user.username)
  })

  it('dirty detects every field', () => {
    const saved = draftFrom(profile)
    expect(isDirty(draftFrom(profile), saved)).toBe(false)
    const changes: Partial<Draft>[] = [
      { avatarHash: null }, { avatarStill: null }, { bannerHash: 'b2' }, { bannerStill: 'b2s' },
      { themeTop: '#000000' }, { themeBottom: '#000000' }, { font: null }, { colors: ['#ffbe7a'] },
      { colors: ['#ffbe7a', '#ff7faf'] }, { pronouns: 'she/her' }, { bio: 'hi' }, { glow: false },
    ]
    for (const c of changes) expect(isDirty({ ...saved, ...c }, saved), JSON.stringify(c)).toBe(true)
  })

  it('empty pronouns and bio become null', () => {
    const d = { ...draftFrom(profile), pronouns: '  ', bio: ' \n ' }
    expect(toRequest(d).pronouns).toBeNull()
    expect(toRequest(d).bio).toBeNull()
    const none = draftFrom({ ...profile, card: { ...profile.card, pronouns: null, bio: null } })
    expect(none.pronouns).toBe('')
    expect(none.bio).toBe('')
  })

  it('previewCard lightens custom colours like the server', () => {
    // a black and a pure blue custom gradient: shown as Save will store them (the shared vectors)
    const d: Draft = { ...draftFrom(profile), colors: ['#000000', '#0000FF'], font: 'orbitron', avatarHash: 'new', avatarStill: null, pronouns: '', bio: '' }
    const u = previewUser(alex, d)
    expect(u.style).toEqual({ font: 'orbitron', colors: ['#a8a8a8', '#a1a1ff'] })
    expect(u.avatar_hash).toBe('new')
    expect(u.avatar_still).toBeNull()
    expect(u.id).toBe('U1')
    expect(u.username).toBe('alex')
    // presets come out unchanged
    expect(previewUser(alex, draftFrom(profile)).style.colors).toEqual(['#ffbe7a', '#ff7fae'])
    // the card: the theme as picked (a custom pair is accepted as is), empty text shows nothing
    const c = previewCard({ ...d, themeTop: '#FFFFFF', themeBottom: '#ffff00' })
    expect(c).toEqual({ banner_hash: 'b1', banner_still: null, theme_top: '#ffffff', theme_bottom: '#ffff00', pronouns: null, bio: null, glow: true })
  })
})

// A literal copy of ZALGO_CASES in server/src/profiles/names.rs (`is_zalgo`): change one side and both tests fail.
const ZALGO_CASES: [string, boolean][] = [
  ['alex', false],
  ['\u{dc}n\u{ef}c\u{f8}d\u{e9} \u{f1}ame', false],
  ['e\u{301}\u{302}', false],
  ['Vie\u{323}\u{302}t', false],
  ['a\u{301} b\u{302}\u{303}', false],
  ['\u{2764}\u{fe0f}\u{fe0f}\u{fe0f}', false],
  ['1\u{fe0f}\u{20e3}', false],
  ['Z\u{335}\u{352}\u{316}', true],
  ['Z\u{335}\u{fe0f}\u{352}\u{316}', true],
  ['kim\u{300}\u{301}\u{302}\u{303}\u{304}\u{305}', true],
  [' \u{300}\u{301}\u{302}', true],
  ['\u{300}\u{301}\u{302}', true],
]

describe('isZalgo (#209)', () => {
  it('is three marks on one character, the server rule', () => {
    expect(ZALGO_MARKS).toBe(3)
    for (const [name, want] of ZALGO_CASES) expect(isZalgo(name), JSON.stringify(name)).toBe(want)
  })
  it('a Zalgo name is still a name', () => {
    expect(nameProblem('Z\u{335}\u{352}\u{316}a\u{334}\u{30d}\u{323}')).toBeNull()
  })
})
