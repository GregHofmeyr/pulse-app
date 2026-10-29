// The profile editor's working copy (spec §4.1): everything is staged here until Save sends it whole.
import { lightenForName } from './nameStyle'
import type { CardProfile } from './protocol/CardProfile'
import type { Profile } from './protocol/Profile'
import type { UpdateProfileRequest } from './protocol/UpdateProfileRequest'
import type { User } from './protocol/User'

export type Draft = {
  avatarHash: string | null
  avatarStill: string | null
  bannerHash: string | null
  bannerStill: string | null
  themeTop: string
  themeBottom: string
  font: string | null
  /** As picked: the server lightens them on Save, the preview the same way. */
  colors: string[]
  pronouns: string
  bio: string
}

export function draftFrom(p: Profile): Draft {
  return {
    avatarHash: p.user.avatar_hash,
    avatarStill: p.user.avatar_still,
    bannerHash: p.card.banner_hash,
    bannerStill: p.card.banner_still,
    themeTop: p.card.theme_top,
    themeBottom: p.card.theme_bottom,
    font: p.user.style.font,
    colors: [...p.user.style.colors],
    pronouns: p.card.pronouns ?? '',
    bio: p.card.bio ?? '',
  }
}

export function isDirty(d: Draft, saved: Draft): boolean {
  const keys = Object.keys(saved) as (keyof Draft)[]
  return keys.some((k) => (k === 'colors' ? d.colors.join() !== saved.colors.join() : d[k] !== saved[k]))
}

const orNull = (s: string) => (s.trim() === '' ? null : s)

export function toRequest(d: Draft): UpdateProfileRequest {
  return {
    avatar_hash: d.avatarHash,
    banner_hash: d.bannerHash,
    theme_top: d.themeTop,
    theme_bottom: d.themeBottom,
    style: { font: d.font, colors: [...d.colors] },
    pronouns: orNull(d.pronouns),
    bio: orNull(d.bio),
  }
}

/** You as the draft would make you, name colours lightened as Save will store them. */
export function previewUser(me: User, d: Draft): User {
  return { ...me, avatar_hash: d.avatarHash, avatar_still: d.avatarStill, style: { font: d.font, colors: d.colors.map(lightenForName) } }
}

/** The card as the draft would make it. A custom theme pair is kept as picked (spec §4.3). */
export function previewCard(d: Draft): CardProfile {
  return {
    banner_hash: d.bannerHash,
    banner_still: d.bannerStill,
    theme_top: d.themeTop.toLowerCase(),
    theme_bottom: d.themeBottom.toLowerCase(),
    pronouns: orNull(d.pronouns)?.trim() ?? null,
    bio: orNull(d.bio),
  }
}

/** Most characters in pronouns and a bio, and most line breaks in a bio (the server's limits). */
export const PRONOUNS_MAX = 40
export const BIO_MAX = 190
export const BIO_LINE_BREAKS_MAX = 4

/** A bio as the server stores it: line endings as `\n`, blank leading lines dropped (the first text
 * line keeps its leading space), the end trimmed. Its characters and line breaks are what count. */
export function normalBio(s: string): string {
  const t = s.replace(/\r\n?/g, '\n')
  const first = t.search(/\S/)
  if (first < 0) return ''
  return t.slice(t.lastIndexOf('\n', first) + 1).trimEnd()
}

/** Characters as the server counts them (Unicode code points, not UTF-16 units). */
export const charCount = (s: string) => [...s].length

/** The fields of `now` that differ from `before` (edits made since `before` was taken). */
export function editsSince(before: Draft, now: Draft): Partial<Draft> {
  const out: Partial<Draft> = {}
  for (const k of Object.keys(before) as (keyof Draft)[]) {
    if (k === 'colors') {
      if (now.colors.join() !== before.colors.join()) out.colors = [...now.colors]
    } else if (now[k] !== before[k]) (out as Record<string, unknown>)[k] = now[k]
  }
  return out
}
