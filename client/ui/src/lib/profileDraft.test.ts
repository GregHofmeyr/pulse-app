import { describe, expect, it } from 'vitest'
import { draftFrom, isDirty, previewCard, previewUser, toRequest, type Draft } from './profileDraft'
import type { Profile } from './protocol/Profile'
import type { User } from './protocol/User'

const alex: User = { id: 'U1', username: 'alex', avatar_hash: 'a1', avatar_still: 'a1s', style: { font: 'fredoka', colors: ['#ffbe7a', '#ff7fae'] } }
const profile: Profile = {
  user: alex,
  card: { banner_hash: 'b1', banner_still: null, theme_top: '#c2457a', theme_bottom: '#3a1f5e', pronouns: 'they/them', bio: 'hi\nthere' },
  stats: { voice_seconds: 0, member_since: '2026-10-03T00:00:00Z', top_reaction: null, times_renamed: 0 },
  badges: [],
  name_archive: [],
}

describe('profileDraft', () => {
  it('round trip draft → request', () => {
    const d = draftFrom(profile)
    expect(d).toEqual({
      avatarHash: 'a1', avatarStill: 'a1s', bannerHash: 'b1', bannerStill: null,
      themeTop: '#c2457a', themeBottom: '#3a1f5e', font: 'fredoka', colors: ['#ffbe7a', '#ff7fae'],
      pronouns: 'they/them', bio: 'hi\nthere',
    })
    expect(toRequest(d)).toEqual({
      avatar_hash: 'a1', banner_hash: 'b1', theme_top: '#c2457a', theme_bottom: '#3a1f5e',
      style: { font: 'fredoka', colors: ['#ffbe7a', '#ff7fae'] }, pronouns: 'they/them', bio: 'hi\nthere',
    })
    // the draft holds its own colours: editing it never touches the profile
    d.colors.push('#000000')
    expect(profile.user.style.colors).toHaveLength(2)
  })

  it('dirty detects every field', () => {
    const saved = draftFrom(profile)
    expect(isDirty(draftFrom(profile), saved)).toBe(false)
    const changes: Partial<Draft>[] = [
      { avatarHash: null }, { avatarStill: null }, { bannerHash: 'b2' }, { bannerStill: 'b2s' },
      { themeTop: '#000000' }, { themeBottom: '#000000' }, { font: null }, { colors: ['#ffbe7a'] },
      { colors: ['#ffbe7a', '#ff7faf'] }, { pronouns: 'she/her' }, { bio: 'hi' },
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
    expect(c).toEqual({ banner_hash: 'b1', banner_still: null, theme_top: '#ffffff', theme_bottom: '#ffff00', pronouns: null, bio: null })
  })
})
