import { describe, expect, it } from 'vitest'
import { chatSize, gifIdOf, gifLink, gifMedia, gifOnly, gifsOf, gridMedia, masonry } from './giphy'
import { shownText } from './poll'
import type { Message } from './protocol/Message'

const ID = '3o7TKSjRrfIPjeiVyM'
const msg = (content: string, over: Partial<Message> = {}): Message => ({
  id: 'M1', channel_id: 'C1', author_id: 'U1', kind: 'normal', content, reply_to_id: null, created_at: '2026-11-11T10:00:00.000Z',
  edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null, embeds: [], ...over,
})

describe('gifIdOf', () => {
  it('reads every GIPHY link form the server accepts', () => {
    for (const link of [
      `https://giphy.com/gifs/excited-yes-${ID}`,
      `https://www.giphy.com/gifs/${ID}`,
      `http://giphy.com/gifs/cat-${ID}/fullscreen`,
      `https://GIPHY.com/gifs/x-${ID}#top`,
      `https://giphy.com/embed/${ID}`,
      `https://media.giphy.com/media/${ID}/giphy.gif`,
      `https://media4.giphy.com/media/${ID}/200w.webp`,
      `https://media2.giphy.com/media/v1.Y2lkPTc5MGI3/${ID}/giphy.webp?cid=1&rid=giphy.webp&ct=g`,
      `https://i.giphy.com/${ID}.webp`,
      `https://i.giphy.com/media/${ID}/giphy.gif`,
    ]) expect(gifIdOf(link), link).toBe(ID)
  })
  it('anything else is none, lookalikes included', () => {
    for (const link of [
      'https://giphy.com/', 'https://giphy.com/explore/cats', 'https://giphy.com/clips/abc-def', 'https://giphy.com.evil.example/gifs/abc',
      'https://evilgiphy.com/gifs/abc', 'https://mediax.giphy.com/media/abc/giphy.gif', 'https://media.giphy.com/media/a%2Fb/giphy.gif',
      'ftp://giphy.com/gifs/abc', 'giphy.com/gifs/abc', 'https://giphy.com/gifs/abc def', 'not a link', '',
    ]) expect(gifIdOf(link), link).toBeNull()
  })
})

describe('gifsOf', () => {
  it('a picked GIF: the message’s own id, title and size', () => {
    const m = msg(`https://giphy.com/gifs/yes-${ID}`, { gif: { id: ID, title: 'Excited Yes GIF', width: 480, height: 270 } })
    expect(gifsOf(m)).toEqual([{ id: ID, title: 'Excited Yes GIF', width: 480, height: 270 }])
  })
  it('pasted links: up to three distinct GIFs, titled from the page link’s words', () => {
    const m = msg(`look https://giphy.com/gifs/happy-dance-${ID}, and https://media.giphy.com/media/abc/giphy.gif and https://giphy.com/gifs/${ID} https://i.giphy.com/d1.gif https://i.giphy.com/d2.gif`)
    expect(gifsOf(m)).toEqual([
      { id: ID, title: 'happy dance', width: null, height: null },
      { id: 'abc', title: '', width: null, height: null },
      { id: 'd1', title: '', width: null, height: null },
    ])
  })
  it('not in code, nor in <angle brackets> (no preview, as for links)', () => {
    expect(gifsOf(msg(`\`https://giphy.com/gifs/${ID}\``))).toEqual([])
    expect(gifsOf(msg(`\`\`\`\nhttps://giphy.com/gifs/${ID}\n\`\`\``))).toEqual([])
    expect(gifsOf(msg(`<https://giphy.com/gifs/${ID}>`))).toEqual([])
  })
  it('a deleted message has none', () => {
    expect(gifsOf(msg('', { deleted: true, gif: { id: ID, title: '', width: 1, height: 1 } }))).toEqual([])
  })
})

describe('gifOnly', () => {
  it('true when the message is just one GIPHY link (its text then hides)', () => {
    expect(gifOnly(msg(`https://giphy.com/gifs/yes-${ID}`))).toBe(true)
    expect(gifOnly(msg(`  https://media.giphy.com/media/${ID}/giphy.gif \n`))).toBe(true)
  })
  it('false with words around it, two links or none', () => {
    expect(gifOnly(msg(`yes https://giphy.com/gifs/yes-${ID}`))).toBe(false)
    expect(gifOnly(msg(`https://giphy.com/gifs/a-${ID} https://giphy.com/gifs/b-abc`))).toBe(false)
    expect(gifOnly(msg('hello'))).toBe(false)
    expect(gifOnly(msg('https://giphy.com/explore/cats'))).toBe(false)
  })
})

describe('one-line previews', () => {
  it('say "GIF" for a message that is only a GIPHY link, the text otherwise', () => {
    expect(shownText(msg(`https://giphy.com/gifs/yes-${ID}`, { gif: { id: ID, title: 'Yes', width: 1, height: 1 } }))).toBe('GIF')
    expect(shownText(msg(`https://media.giphy.com/media/${ID}/giphy.gif`))).toBe('GIF')
    expect(shownText(msg(`ha https://giphy.com/gifs/yes-${ID}`))).toBe(`ha https://giphy.com/gifs/yes-${ID}`)
  })
})

describe('media and sizes', () => {
  it('GIPHY’s own media links for an id: 200 px high in chat, 200 px wide in the grid', () => {
    expect(gifMedia(ID)).toEqual({ anim: `https://media.giphy.com/media/${ID}/200.webp`, still: `https://media.giphy.com/media/${ID}/200_s.gif` })
    expect(gridMedia(ID)).toEqual({ anim: `https://media.giphy.com/media/${ID}/200w.webp`, still: `https://media.giphy.com/media/${ID}/200w_s.gif` })
  })
  it('chat size: 200 px high, at most 300 wide, by the GIF’s shape; unknown shape: none', () => {
    expect(chatSize({ width: 480, height: 270 })).toEqual({ width: 300, height: 169 })
    expect(chatSize({ width: 270, height: 480 })).toEqual({ width: 113, height: 200 })
    expect(chatSize({ width: 200, height: 200 })).toEqual({ width: 200, height: 200 })
    expect(chatSize({ width: null, height: null })).toBeNull()
  })
  it('the link a picked GIF is sent as: GIPHY’s page for it, else the plain one', () => {
    expect(gifLink({ id: ID, url: `https://giphy.com/gifs/yes-${ID}` })).toBe(`https://giphy.com/gifs/yes-${ID}`)
    expect(gifLink({ id: ID, url: 'https://giphy.com/gifs/other-abc' })).toBe(`https://giphy.com/gifs/${ID}`)
    expect(gifLink({ id: ID, url: '' })).toBe(`https://giphy.com/gifs/${ID}`)
  })
})

describe('masonry', () => {
  it('two columns, each GIF to the shorter one, in order', () => {
    const items = [
      { id: 'a', h: 100 }, { id: 'b', h: 50 }, { id: 'c', h: 60 }, { id: 'd', h: 10 },
    ]
    const cols = masonry(items, (i) => i.h)
    expect(cols.map((c) => c.map((i) => i.id))).toEqual([['a', 'd'], ['b', 'c']])
  })
})
