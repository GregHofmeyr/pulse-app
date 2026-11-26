import { describe, expect, it } from 'vitest'
import { applyEvent, applyReady, emptyState, TYPING_STOP_GUARD_MS, TYPING_TTL_MS, type AppState } from './state'
import { typingPeople } from './selectors'
import { swapTracker, TYPING_SUGGESTIONS, TYPING_TEXT_MAX, typingLine, typingTextProblem } from './typingLine'
import type { Message } from './protocol/Message'
import type { Ready } from './protocol/Ready'
import type { User } from './protocol/User'

const person = (id: string, name: string, typing_text?: string): User => ({ id, username: name, display_name: name, avatar_hash: null, avatar_still: null, style: { font: null, colors: [] }, ...(typing_text ? { typing_text } : {}) })
const alex = person('U1', 'alex')
const sam = person('U2', 'sam', 'cooking')
const kim = person('U3', 'kim', 'barking')
const jo = person('U4', 'jo')

const ready = (): Ready => ({
  me: alex, servers: [{ id: 'S1', name: 'Main', icon_hash: null }],
  channels: [{ id: 'C1', server_id: 'S1', kind: 'text', name: 'general', position: 0 }],
  members: [{ server_id: 'S1', members: [alex, sam, kim, jo].map((user) => ({ user, nickname: null })) }],
  dm_members: [], voice: [], people: [], read_states: [], mutes: [], hidden: [], latest: [], streams: [], custom_emoji: [], frequent_emoji: [], qoty: [], speaker_links: [], thread_seen: [], games: [], clip_views: [], private_nicknames: [], sounds: [], sound_favourites: [], frequent_sounds: [], server_voice: [], status: { status: 'online', dnd_until: null }, appear_offline_to: [], live_notify: [],
} as unknown as Ready)

const typing = (s: AppState, user: string, at: number) => applyEvent(s, { t: 'Typing', d: { channel_id: 'C1', user_id: user } }, at)
const sent = (s: AppState, user: string, at: number, id = `M${at}`) => {
  const message = { id, channel_id: 'C1', author_id: user, kind: 'normal', content: 'hi', reply_to_id: null, created_at: '2026-11-26T00:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null, embeds: [] } as unknown as Message
  return applyEvent(s, { t: 'MessageCreated', d: { message, nonce: null } }, at)
}
/** The line as the indicator shows it, as one string, or '' when nobody is typing. */
const shown = (s: AppState, at: number) => {
  const l = typingLine(typingPeople(s, 'S1', 'C1', at))
  return l ? `${l.names} ${l.rest}` : ''
}

describe('typingLine', () => {
  it('one typer with typing text: "<name> is <text>"', () => {
    expect(typingLine([{ id: 'U2', name: 'sam', custom: 'cooking' }])).toEqual({ key: 'sam\u0000is cooking', names: 'sam', rest: 'is cooking' })
  })

  it('one typer without: "<name> is typing…"', () => {
    expect(typingLine([{ id: 'U4', name: 'jo', custom: null }])).toMatchObject({ names: 'jo', rest: 'is typing…' })
  })

  it("two or more typers: the default, whatever anyone's typing text", () => {
    expect(typingLine([{ id: 'U2', name: 'sam', custom: 'cooking' }, { id: 'U3', name: 'kim', custom: 'barking' }])).toMatchObject({ names: 'sam, kim', rest: 'are typing…' })
    expect(typingLine([{ id: 'U2', name: 'sam', custom: 'cooking' }, { id: 'U3', name: 'kim', custom: null }, { id: 'U4', name: 'jo', custom: null }])).toMatchObject({ names: 'sam, kim, jo', rest: 'are typing…' })
  })

  it('nobody typing: no line', () => {
    expect(typingLine([])).toBeNull()
  })

  it('the key changes with the words, so a change crossfades and the same line stays put', () => {
    const a = typingLine([{ id: 'U2', name: 'sam', custom: 'cooking' }])!
    expect(typingLine([{ id: 'U2', name: 'sam', custom: 'cooking' }])!.key).toBe(a.key)
    expect(typingLine([{ id: 'U2', name: 'sam', custom: null }])!.key).not.toBe(a.key)
    expect(typingLine([{ id: 'U2', name: 'kim', custom: 'cooking' }])!.key).not.toBe(a.key)
  })
})

describe('typingPeople', () => {
  it("carries each typer's typing text from the user already held (no fetch)", () => {
    let s = applyReady(emptyState(), ready())
    s = typing(s, 'U2', 0)
    s = typing(s, 'U4', 0)
    expect(typingPeople(s, 'S1', 'C1', 1)).toEqual([{ id: 'U2', name: 'sam', custom: 'cooking' }, { id: 'U4', name: 'jo', custom: null }])
  })

  it('follows a profile update', () => {
    let s = typing(applyReady(emptyState(), ready()), 'U2', 0)
    s = applyEvent(s, { t: 'ProfileUpdated', d: { user: person('U2', 'sam', 'yapping'), card: { banner_hash: null, banner_still: null, theme_top: '#000000', theme_bottom: '#000000', pronouns: null, bio: null, glow: true } } }, 0)
    expect(shown(s, 1)).toBe('sam is yapping')
  })
})

describe('typing races', () => {
  const start = () => applyReady(emptyState(), ready())

  it("sam typing → kim starts → sam stops: kim's own text shows", () => {
    let s = typing(start(), 'U2', 0)
    expect(shown(s, 10)).toBe('sam is cooking')
    s = typing(s, 'U3', 100)
    expect(shown(s, 110)).toBe('sam, kim are typing…')
    s = sent(s, 'U2', 200)
    expect(shown(s, 210)).toBe('kim is barking')
  })

  it("sam typing → kim starts → sam times out: kim's own text shows, then nothing", () => {
    let s = typing(start(), 'U2', 0)
    s = typing(s, 'U3', 2000)
    expect(shown(s, 2010)).toBe('sam, kim are typing…')
    expect(shown(s, TYPING_TTL_MS + 1)).toBe('kim is barking')
    expect(shown(s, 2000 + TYPING_TTL_MS + 1)).toBe('')
  })

  it('a typer with no text joining one with text: the default for both, then back', () => {
    let s = typing(start(), 'U2', 0)
    s = typing(s, 'U4', 100)
    expect(shown(s, 110)).toBe('sam, jo are typing…')
    s = sent(s, 'U2', 200)
    expect(shown(s, 210)).toBe('jo is typing…')
  })

  it("out of order: a Typing that lands just after that person's message brings no ghost line back", () => {
    let s = typing(start(), 'U2', 0)
    s = sent(s, 'U2', 100) // their message overtook their last Typing
    s = typing(s, 'U2', 150)
    expect(shown(s, 160)).toBe('')
    // also when the message landed first and their Typing never had
    s = sent(start(), 'U3', 0)
    s = typing(s, 'U3', 50)
    expect(shown(s, 60)).toBe('')
  })

  it('a stale Typing from one person leaves the others alone', () => {
    let s = typing(start(), 'U3', 0)
    s = typing(s, 'U2', 10)
    s = sent(s, 'U2', 100)
    s = typing(s, 'U2', 120) // late
    expect(shown(s, 130)).toBe('kim is barking')
  })

  it('typing again once the guard has passed shows them again', () => {
    let s = sent(start(), 'U2', 0)
    s = typing(s, 'U2', TYPING_STOP_GUARD_MS + 1)
    expect(shown(s, TYPING_STOP_GUARD_MS + 2)).toBe('sam is cooking')
  })

  it('typing events in any order give the same line: it is worked out from who is typing now, not from the events', () => {
    const a = typing(typing(start(), 'U2', 0), 'U3', 10)
    const b = typing(typing(start(), 'U3', 0), 'U2', 10)
    expect(shown(a, 20)).toBe('sam, kim are typing…')
    expect(shown(b, 20)).toBe('kim, sam are typing…')
    // a repeat Typing keeps the order (no names swapping places)
    expect(shown(typing(a, 'U2', 30), 40)).toBe('sam, kim are typing…')
  })

  it('the guard memory stays small: old stops are dropped', () => {
    let s = start()
    for (let i = 0; i < 50; i++) s = sent(s, 'U2', i * (TYPING_STOP_GUARD_MS + 1), `M${i}`)
    s = sent(s, 'U3', 50 * (TYPING_STOP_GUARD_MS + 1), 'Mx')
    expect(Object.keys(s.typingStops['C1'])).toEqual(['U3'])
  })
})

describe('swapTracker', () => {
  it('says whether a new line replaces one on screen (crossfade) or appears from nothing (rise)', () => {
    const swap = swapTracker()
    expect(swap(null)).toBe(false)
    expect(swap('a')).toBe(false) // appears
    expect(swap('a')).toBe(false) // same line
    expect(swap('b')).toBe(true) // changed words
    expect(swap(null)).toBe(false) // gone
    expect(swap('c')).toBe(false) // appears again
  })
})

describe('typing text rules (the server keeps them; this is the preview)', () => {
  it('blank is fine: the plain "is typing"', () => {
    expect(typingTextProblem('')).toBeNull()
    expect(typingTextProblem('   ')).toBeNull()
  })

  it('up to 32 characters, counted as the server counts', () => {
    expect(TYPING_TEXT_MAX).toBe(32)
    expect(typingTextProblem('x'.repeat(32))).toBeNull()
    expect(typingTextProblem('é'.repeat(32))).toBeNull()
    expect(typingTextProblem('x'.repeat(33))).toBe('Typing text can be up to 32 characters.')
  })

  it('no line breaks, controls, invisible characters or links (the server messages)', () => {
    expect(typingTextProblem('a\nb')).toBe("Typing text can't contain line breaks or control characters.")
    expect(typingTextProblem('\u202egnippay')).toBe("Typing text can't contain line breaks or control characters.")
    expect(typingTextProblem('yap\u200bping')).toBe("Typing text can't contain invisible characters.")
    expect(typingTextProblem('\u200dbarking')).toBe("Typing text can't contain invisible characters.")
    expect(typingTextProblem('cooking 👩\u200d🍳')).toBeNull()
    for (const link of ['https://example.com', 'www.example', 'on pulse.app']) expect(typingTextProblem(link)).toBe("Typing text can't contain links.")
    for (const ok of ['typing a novel...', 'on v1.2', '3.14']) expect(typingTextProblem(ok)).toBeNull()
  })

  it('about six suggestions, each one that passes', () => {
    expect(TYPING_SUGGESTIONS).toHaveLength(6)
    for (const t of TYPING_SUGGESTIONS) expect(typingTextProblem(t)).toBeNull()
    expect(TYPING_SUGGESTIONS).toEqual(expect.arrayContaining(['cooking', 'yapping', 'plotting', 'barking', 'typing a novel']))
  })
})

describe('typing again right after a send (#652 review)', () => {
  it('kim sends, then types 300 ms later: others see her typing within about 0.7 s, and it never drops out', async () => {
    const { typingThrottle, typingSentAfterSend } = await import('./markdown')
    const LATENCY = 40
    // the sender's side, as the Composer does it: one Typing per allowed keystroke, a keystroke every 50 ms
    let last: number | null = 0
    const typingsAt: number[] = []
    last = typingSentAfterSend(0) // the send at t=0
    for (let t = 300; t <= 4000; t += 50) {
      if (typingThrottle(last, t)) {
        last = t
        typingsAt.push(t)
      }
    }
    // the watcher's side: the message lands, then each Typing, all a little later
    let s = sent(applyReady(emptyState(), ready()), 'U3', LATENCY)
    const at = (t: number) => {
      for (const sentAt of typingsAt) if (sentAt + LATENCY <= t && !seen.has(sentAt)) { seen.add(sentAt); s = typing(s, 'U3', sentAt + LATENCY) }
      return shown(s, t)
    }
    const seen = new Set<number>()
    expect(at(600)).toBe('') // not yet
    for (let t = 700; t <= 4000; t += 20) expect(at(t), `at ${t} ms`).toBe('kim is barking')
  })
})

/** A literal copy of TYPING_TEXT_CASES in server/src/profiles/names.rs (#652 review): change one side and both tests
 *  fail, so the server never refuses what this preview let through. null: accepted. */
const INVISIBLE = "Typing text can't contain invisible characters."
const CONTROL = "Typing text can't contain line breaks or control characters."
const LINK = "Typing text can't contain links."
const TYPING_TEXT_CASES: [string, string | null][] = [
  ['cooking', null],
  ['cooking 👩\u{200d}🍳', null],
  ['from 🏴\u{e0067}\u{e0062}\u{e0073}\u{e0063}\u{e0074}\u{e007f}', null],
  ['hearts ❤\u{fe0f}', null],
  ['typing a novel...', null],
  ['on v1.2', null],
  ['3.14', null],
  ['cooking. again', null],
  ['\u{200b}', INVISIBLE],
  ['yap\u{200b}ping', INVISIBLE],
  ['\u{200d}barking', INVISIBLE],
  ['a \u{200d} b', INVISIBLE],
  ['yap\u{61c}ping', INVISIBLE],
  ['yap\u{e0020}ping', INVISIBLE],
  ['yap\u{e0041}ping', INVISIBLE],
  ['yap\u{e007f}ping', INVISIBLE],
  ['yap\u{ad}ping', INVISIBLE],
  ['yap\u{2800}ping', INVISIBLE],
  ['\u{2800}', INVISIBLE],
  ['yap\u{180b}ping', INVISIBLE],
  ['yap\u{180c}ping', INVISIBLE],
  ['yap\u{180d}ping', INVISIBLE],
  ['yap\u{180f}ping', INVISIBLE],
  ['yap\u{fff9}ping', INVISIBLE],
  ['yap\u{fffa}ping', INVISIBLE],
  ['yap\u{fffb}ping', INVISIBLE],
  ['a\nb', CONTROL],
  ['\u{202e}gnippay', CONTROL],
  ['https://example.com', LINK],
  ['www.example', LINK],
  ['WWW.EXAMPLE', LINK],
  ['on pulse.app', LINK],
  ['pulse\u{3002}app', LINK],
  ['pulse\u{ff0e}app', LINK],
  ['pulse\u{ff61}app', LINK],
  ['pulse\u{2024}app', LINK],
  ['pulse\u{2027}app', LINK],
  ['pulse\u{fe52}app', LINK],
  ['пример.рф', LINK],
  ['brain.exe', LINK],
]

describe('typing text rules match the server, case for case', () => {
  it.each(TYPING_TEXT_CASES)('%j', (text, want) => {
    expect(typingTextProblem(text)).toBe(want)
  })
})

it('the new invisible characters are never all of a name (names.rs, the same set)', async () => {
  const { nameProblem } = await import('./profileDraft')
  for (const bad of ['\u{2800}', '\u{ad}', '\u{180b}\u{180f}']) expect(nameProblem(bad), bad).toContain('see')
})
