// Zalgo names (#209): a name's stacked combining marks stay inside its own line box, wherever a name shows, so they
// never cover the rows around it. Names are allowed to be Zalgo; this checks each place a name renders clips it.
// (jsdom has no layout: what it looks like was checked by rendering a heavy-Zalgo name in Chromium.)
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

const src = (p: string) => readFileSync(resolve(__dirname, p), 'utf8')
const css = src('app.css')

describe('Zalgo names are clipped to their own line (#209)', () => {
  it('one shared clip: a token and the .name-clip utility', () => {
    expect(css).toMatch(/--name-clip:\s*inset\(-\.15em -\.1em\);/)
    expect(css).toMatch(/\.name-clip\s*\{\s*clip-path:\s*var\(--name-clip\);\s*\}/)
  })

  it('Name.svelte, every styled name, clips to its line box', () => {
    const name = src('components/Name.svelte')
    expect(name).toMatch(/\.name \{[^}]*overflow: hidden;/)
    expect(name).toMatch(/\.name \{[^}]*line-height: calc\(1\.4em/)
  })

  it('the tooltip chip clips what it holds', () => {
    expect(css).toMatch(/\.pulse-tooltip \{[^}]*overflow: clip;/)
  })

  it('mention pills in messages and the message box clip', () => {
    expect(src('components/MessageItem.svelte')).toMatch(/:global\(\.mention\) \{[^}]*clip-path: var\(--name-clip\);/)
    expect(src('components/EmojiBox.svelte')).toMatch(/:global\(\.mention-chip\) \{[^}]*clip-path: var\(--name-clip\);/)
  })

  // Names written as plain text (not through Name.svelte) carry the utility.
  const plain: [string, RegExp][] = [
    ['components/MemberList.svelte', /<small class="name-clip">\{base\}<\/small>/],
    ['components/TextChannel.svelte', /<strong class="name-clip">\{typers\.join/],
    ['components/Composer.svelte', /Replying to <strong class="name-clip">\{replyingTo\}<\/strong>/],
    ['components/CallRingPopover.svelte', /<strong class="name-clip">\{caller\}<\/strong>/],
    ['components/CallRingPopover.svelte', /<span class="name-clip">\{caller\}<\/span> is calling/],
    ['components/ConversationList.svelte', /<span class="name-clip">\{c\.name\}<\/span>/],
    ['components/FullProfile.svelte', /<strong class="name-clip">\{e\.name\}<\/strong>/],
    ['components/QuoteItem.svelte', /<span class="name-clip">\{who\.name\}<\/span>/],
    ['components/PeoplePicker.svelte', /<span class="name-clip">\{nameOf\(id\)\}<\/span>/],
    ['components/BadgeToast.svelte', /<span class="name-clip">\{t\.name\}<\/span>/],
    ['components/TrophyCase.svelte', /<span class="nm name-clip">/],
  ]
  it.each(plain)('%s clips its plain-text names', (file, re) => {
    expect(src(file)).toMatch(re)
  })
})
