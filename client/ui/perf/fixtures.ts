// Realistic volumes for the perf harness (#238): two servers with many channels and members, and channels whose
// history has ~200 messages with markdown, mentions, embeds, reactions, replies and quotes. Test people only.
import type { Channel } from '../src/lib/protocol/Channel'
import type { Message } from '../src/lib/protocol/Message'
import type { Member } from '../src/lib/protocol/Member'
import type { Person } from '../src/lib/protocol/Person'
import type { Ready } from '../src/lib/protocol/Ready'
import type { User } from '../src/lib/protocol/User'
import { testReady } from '../src/lib/testReady'

const NAMES = ['alex', 'sam', 'kim', 'jo']
const FONTS = [null, null, 'pacifico', 'orbitron', 'caveat']
const pad = (n: number, w = 8) => String(n).padStart(w, '0')

export const me: User = { id: 'U0000', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }

/** 400 people: the test names with a number. A few have name styles. */
export const users: User[] = [me]
for (let i = 1; i < 400; i++) {
  const base = NAMES[i % NAMES.length]
  users.push({
    id: `U${pad(i, 4)}`, username: `${base}${i}`, display_name: `${base} ${i}`, avatar_hash: null, avatar_still: null,
    style: i % 7 === 0 ? { font: FONTS[i % FONTS.length], colors: i % 14 === 0 ? ['#ff6b6b', '#4dabf7'] : ['#ffa94d'] } : { font: null, colors: [] },
  })
}

function channelsFor(server: string, n: number): Channel[] {
  const out: Channel[] = []
  for (let i = 0; i < n; i++) {
    const kind = i === 3 ? 'quotes' : i % 6 === 5 ? 'voice' : 'text'
    out.push({ id: `${server}-C${pad(i, 3)}`, server_id: server, kind, name: kind === 'quotes' ? 'quotes' : `${kind === 'voice' ? 'voice' : 'chan'}-${i}`, position: i })
  }
  return out
}
export const channels: Channel[] = [...channelsFor('S1', 40), ...channelsFor('S2', 25)]

const people: Person[] = users.map((user, i) => ({ user, online: i % 3 !== 0, last_seen_at: null, status: i % 3 === 0 ? 'offline' : i % 5 === 0 ? 'idle' : 'online' }) as Person)
const members = (from: number, to: number): Member[] => users.slice(from, to).map((user) => ({ user, nickname: null }))

export const ready: Ready = testReady(channels, {
  me,
  servers: [{ id: 'S1', name: 'Main', icon_hash: null }, { id: 'S2', name: 'Games', icon_hash: null }],
  members: [{ server_id: 'S1', members: members(0, 300) }, { server_id: 'S2', members: members(0, 150) }],
  people,
  read_states: channels.map((c) => ({ channel_id: c.id, last_read_message_id: msgId(c.id, 190), unread: 10, mentions: 0 })),
})

function msgId(channel: string, i: number) {
  // sortable, like the server's ids
  return `${channel}-M${pad(i)}`
}

const HASH = 'a'.repeat(64)
const TEXTS = [
  'ok sounds good',
  'did anyone try the new patch? the **voice** feels a lot smoother on my end',
  'here: https://example.com/articles/how-to-tune-noise-suppression and the `--strong` flag',
  'lol 😂😂',
  '@sam can you check this when you get a sec?',
  '```rust\nfn main() {\n    println!("hello");\n}\n```',
  '> quoting the docs here\nwhich says it should *just work*',
  'Three things:\n- first\n- second, with a [link](https://example.org/a)\n- third ~~struck~~',
  'https://www.youtube.com/watch?v=dQw4w9WgXcQ',
  'brb',
  "that's a long message that wraps over a couple of lines so the layout has some real work to do, with **bold**, _italics_, `code` and an emoji 🎉 near the end",
  '@kim @jo meeting at 8?',
]

function message(channel: string, i: number, quotes: boolean): Message {
  const author = users[1 + ((i * 7) % 30)]
  const text = TEXTS[i % TEXTS.length]
  const m: Message = {
    id: msgId(channel, i), channel_id: channel, author_id: author.id, kind: 'normal', content: text,
    reply_to_id: i % 10 === 4 && i > 2 ? msgId(channel, i - 2) : null,
    created_at: new Date(Date.UTC(2026, 10, 1, 10, 0, 0) + i * 47_000).toISOString(), edited_at: i % 17 === 0 ? new Date(Date.UTC(2026, 10, 1, 12)).toISOString() : null,
    deleted: false, mentions: text.includes('@') ? ['U0002'] : [],
    reactions: i % 3 === 0 ? [{ emoji: '👍', count: 1 + (i % 4), me: i % 2 === 0 }, ...(i % 6 === 0 ? [{ emoji: '😂', count: 2, me: false }, { emoji: '🎉', count: 1, me: false }] : [])] : [],
    thread_root_id: null, quote: null, qoty_card: null, clip: null, embeds: [],
  }
  if (text.includes('example.com/articles')) {
    m.embeds = [{ url: 'https://example.com/articles/how-to-tune-noise-suppression', provider: 'generic', kind: 'card', provider_id: null, title: 'How to tune noise suppression', description: 'A long description of the article that goes on for a while, as these do, so it wraps over two lines in the card.', site_name: 'Example', author: null, thumb: { hash: HASH, width: 400, height: 225 }, start: null, vertical: false }]
  } else if (text.includes('youtube')) {
    m.embeds = [{ url: 'https://www.youtube.com/watch?v=dQw4w9WgXcQ', provider: 'youtube', kind: 'player', provider_id: 'dQw4w9WgXcQ', title: 'A video', description: null, site_name: 'YouTube', author: 'someone', thumb: { hash: HASH, width: 480, height: 360 }, start: null, vertical: false }]
  }
  if (quotes) {
    m.content = ''
    m.quote = {
      lines: [
        { id: `${m.id}-L1`, speaker_id: author.id, speaker_name: null, text: TEXTS[(i + 3) % TEXTS.length].replace(/```[^]*```/, 'code'), media: null, caption: null, awards: i % 4, awarders: [], awarded: false },
        ...(i % 2 ? [{ id: `${m.id}-L2`, speaker_id: null, speaker_name: 'jo', text: 'no way', media: null, caption: null, awards: 0, awarders: [], awarded: false }] : []),
      ],
      year: 2026, said_on: null, reply_count: i % 5, last_reply_id: null, replies_seen: null, wins: [],
    }
  }
  return m
}

/** Each channel's history: `count` messages, newest last. */
const histories = new Map<string, Message[]>()
export function history(channel: string, count: number): Message[] {
  let h = histories.get(channel)
  if (!h) {
    const quotes = channels.find((c) => c.id === channel)?.kind === 'quotes'
    h = Array.from({ length: count }, (_, i) => message(channel, i, quotes))
    histories.set(channel, h)
  }
  return h
}
