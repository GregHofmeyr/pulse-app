/// <reference types="node" />
// One presence function (#45, spec "How it gets made"): every dot and "last seen" is drawn from presenceOf, so
// Invisible, DND and its timer look the same everywhere. This scan pins the places that draw one, and trips if
// anything else starts reading `online` / `last_seen_at` to draw.
import { readdirSync, readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'

const src = new URL('../', import.meta.url)
const sources = (readdirSync(src, { recursive: true }) as string[])
  .filter((f) => /\.(ts|svelte)$/.test(f) && !/\.test\.ts$/.test(f) && !f.startsWith('lib/protocol/'))
  .map((f) => ({ f, text: readFileSync(new URL(f, src), 'utf8') }))

/** The places that draw a presence dot or "last seen" (owner-visible list: add to it on purpose). */
const DRAWS = [
  'components/ConversationList.svelte', // a 1:1 DM's avatar
  'components/GroupMembers.svelte', // a group's member list
  'components/InviteDialog.svelte', // picking people to invite to a server (#80)
  'components/MemberList.svelte', // a server's member list (#169)
  'components/PeopleBoard.svelte', // "Your people": dots, the online/offline split, last seen
  'components/PeoplePicker.svelte', // picking people for a group
  'components/ProfileCard.svelte', // the card's avatar
  'components/TextChannel.svelte', // a DM's header
  'routes/Shell.svelte', // your own avatar, bottom of the sidebar
]

describe('presence is drawn in one way', () => {
  it('the dot is drawn exactly in the pinned places, each from presenceOf', () => {
    const drawing = sources.filter(({ text }) => /<Avatar\b[^>]*(\bpresence=|\{presence\})/.test(text)).map(({ f }) => f).sort()
    expect(drawing).toEqual(DRAWS)
    for (const { f, text } of sources.filter(({ f }) => DRAWS.includes(f))) {
      expect(text, f).toMatch(/import \{[^}]*\bpresenceOf\b[^}]*\} from '..\/lib\/presence'/)
    }
  })

  it('nothing outside lib/presence.ts reads online or last_seen_at to draw', () => {
    // state.ts stores them (from Ready and PresenceChanged); status.ts reads your own status; presence.ts draws
    const allowed = ['lib/presence.ts', 'lib/state.ts', 'lib/status.ts']
    const readers = sources
      .filter(({ f }) => !allowed.includes(f))
      .filter(({ text }) => /\.online\b|\blast_seen_at\b|\.status\s*===?\s*'(online|dnd|offline)'/.test(text))
      .map(({ f }) => f)
    expect(readers).toEqual([])
  })

  it('every dot site draws every kind: the avatar dot (and your card\'s status row) style each one', () => {
    // 'online' is the dot's own colour; each other kind needs its rule, or it would look online
    const kinds = ['idle', 'dnd', 'offline', 'invisible'] // keep in step with PresenceKind
    const presence = sources.find(({ f }) => f === 'lib/presence.ts')!.text
    expect(presence).toMatch(new RegExp(`PresenceKind = 'online'${kinds.map((k) => ` \\| '${k}'`).join('')}`))
    const avatar = sources.find(({ f }) => f === 'components/Avatar.svelte')!.text
    const card = sources.find(({ f }) => f === 'components/ProfileCard.svelte')!.text
    for (const k of kinds) {
      expect(avatar, k).toMatch(new RegExp(`\\.pres\\.${k}\\b`))
      expect(card, k).toMatch(new RegExp(`\\.status\\.${k === 'offline' ? 'invisible' : k}\\b`))
    }
  })

  it('the scan sees the sources', () => {
    expect(sources.some(({ f }) => f === 'lib/presence.ts')).toBe(true)
    expect(sources.some(({ f }) => f === 'routes/Shell.svelte')).toBe(true)
  })
})
