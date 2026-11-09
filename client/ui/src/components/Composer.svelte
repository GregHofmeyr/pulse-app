<script lang="ts" module>
  // Unsent text per channel, kept while the app runs (switching channels doesn't lose it).
  const drafts = new Map<string, string>()
</script>

<script lang="ts">
  import { untrack } from 'svelte'
  import Icon from './Icon.svelte'
  import EmojiBox, { type Snap } from './EmojiBox.svelte'
  import EmojiPopover from './EmojiPopover.svelte'
  import WheelPopover from './WheelPopover.svelte'
  import { customToken, savedTone, search, twemojiUrl, withSkin, type Std } from '../lib/emoji'
  import { applyEmoji, emojiQuery, insertAt } from '../lib/emojiComplete'
  import { animatedEmoji, stillSrc } from '../lib/animate.svelte'
  import { typingThrottle } from '../lib/markdown'
  import { applyMention, mentionQuery, suggest, type Mentionable } from '../lib/mentions'
  import { applyCommand, commandQuery, suggestCommands, typingCommand, type SlashCommand } from '../lib/commands'
  import type { CustomEmoji } from '../lib/protocol/CustomEmoji'
  import { memberServers } from '../lib/reactions'
  import { app } from '../lib/store.svelte'
  import { displayName } from '../lib/selectors'
  import Name from './Name.svelte'

  let {
    placeholder,
    draftKey,
    replyingTo,
    onCancelReply,
    onSend,
    onTyping,
    mentionables = [],
    serverId = null,
    wheelChannel = null,
    commands = [],
  }: {
    placeholder: string
    draftKey: string
    replyingTo: string | null
    onCancelReply: () => void
    onSend: (text: string) => void
    onTyping: () => void
    /** People who can be @mentioned here (autocomplete). */
    mentionables?: Mentionable[]
    /** The server it posts in (null in DMs and groups): suggested names use its role colours. */
    serverId?: string | null
    /** The chat a picker wheel spins in (#158): shows the wheel button. None where wheels don't spin (threads). */
    wheelChannel?: string | null
    /** Slash commands that run here (#262, lib/commands.ts): suggested on `/`, with a usage tip. None where they don't. */
    commands?: readonly SlashCommand[]
  } = $props()

  // The composer remounts per channel ({#key} in Shell), so reading the key once is intended.
  let text = $state(untrack(() => drafts.get(draftKey) ?? ''))
  $effect(() => {
    if (text) drafts.set(draftKey, text)
    else drafts.delete(draftKey)
  })
  let lastTyping: number | null = null
  // The message box shows custom emoji as images; `text` (what's sent and what drafts keep) stays the truth.
  let box: EmojiBox

  // @-mention popup: open while the caret sits right after an @token that matches someone.
  let caret = $state(0)
  let pick = $state(0)
  const query = $derived(mentionQuery(text.slice(0, caret)))
  const options = $derived(query === null ? [] : suggest(mentionables, query))
  let dismissed = $state<string | null>(null) // Esc closes the popup until the token changes
  const open = $derived(options.length > 0 && dismissed !== query)

  // `:name` popup: custom emoji you can use (your servers) and standard ones. Never open with @mentions:
  // the two queries can't both end at the caret.
  const emojiQ = $derived(emojiQuery(text.slice(0, caret)))
  const usable = $derived.by(() => {
    const mine = memberServers(app.state)
    return Object.values(app.state.customEmoji).filter((c) => mine.has(c.server_id))
  })
  const emojiOptions = $derived(emojiQ === null ? [] : search(emojiQ, usable).slice(0, 8))
  // Re-read as the query changes: a tone picked in the picker meanwhile applies from the next keystroke.
  const tone = $derived(emojiOptions.length ? savedTone() : 0)
  const emojiOpen = $derived(emojiOptions.length > 0 && dismissed !== ':' + emojiQ)
  // `/name` starting the message (#262): the commands it could be. Never with the two above: a command name is
  // one word right after the `/`, so neither an @ nor a : can end at the caret.
  const cmdQ = $derived(commands.length ? commandQuery(text, caret) : null)
  const cmdOptions = $derived(cmdQ === null ? [] : suggestCommands(cmdQ, commands))
  const cmdOpen = $derived(cmdOptions.length > 0 && dismissed !== '/' + cmdQ)
  // the usage tip while a command's arguments are typed; any list showing takes its place
  const typing = $derived(commands.length ? typingCommand(text, commands) : null)
  const shown = $derived(open ? options.length : emojiOpen ? emojiOptions.length : cmdOpen ? cmdOptions.length : 0)
  const serverName = (id: string) => app.state.servers.find((s) => s.id === id)?.name ?? ''
  const emojiText = (e: Std | CustomEmoji) => ('u' in e ? withSkin(e, tone) : customToken(e))

  let picking = $state(false)
  let emojiBtn: HTMLButtonElement | undefined = $state()
  // the wheel popover (#158); `wheelPrefill`: options typed after `/wheel` (fewer than two)
  let wheeling = $state(false)
  let wheelPrefill = $state<string[] | null>(null)
  let wheelBtn: HTMLButtonElement | undefined = $state()

  function syncCaret() {
    caret = box.readSel()?.start ?? text.length
    pick = 0
  }

  /** Replace the text (an emoji or mention picked) and put the caret at `r.caret` (the box reports it).
   *  Without `focus` (a Shift-pick in the picker) focus stays where it is and the caret waits in the box. */
  const setText = (r: Snap, focus = true) => void box.setText(r, focus)

  // a mention is written as the username (what the server reads); the box shows it as a chip with
  // the name that applies here (#47)
  const choose = (username: string) => setText(applyMention(text, caret, username))
  const mentionName = (username: string) => {
    const u = username.toLowerCase()
    const me = app.state.me
    if (me && me.username.toLowerCase() === u) return displayName(app.state, serverId, me.id)
    return mentionables.find((m) => m.username.toLowerCase() === u)?.name ?? null
  }
  const chooseEmoji = (e: Std | CustomEmoji) => setText(applyEmoji(text, caret, emojiText(e)))
  const chooseCommand = (c: SlashCommand) => setText(applyCommand(text, caret, c.name))

  // `sel` keeps the box's selection while the picker has focus: insert there.
  // A `keep` pick (Shift) leaves the picker open and the box unfocused, with the caret advanced.
  function pickEmoji(e: Std | CustomEmoji, { keep }: { keep: boolean }) {
    if (!keep) picking = false
    const sel = box.selection()
    void setText(insertAt(text, sel.start, sel.end, emojiText(e)), !keep)
  }

  // after the box's own keys (IME, undo/redo); Shift+Enter not taken here adds a line
  function key(e: KeyboardEvent) {
    if (shown) {
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault()
        pick = (pick + (e.key === 'ArrowDown' ? 1 : shown - 1)) % shown
        return
      }
      // Enter on a command already typed in full (`/wheel`) runs it as sent; Tab still completes it
      const runs = e.key === 'Enter' && !open && !emojiOpen && cmdOptions[pick]?.name === cmdQ?.toLowerCase()
      if ((e.key === 'Enter' || e.key === 'Tab') && !runs) {
        e.preventDefault()
        // a pick that doesn't fit isn't made (setText refuses it)
        void (open ? choose(options[pick].username) : emojiOpen ? chooseEmoji(emojiOptions[pick]) : chooseCommand(cmdOptions[pick]))
        return
      }
      if (e.key === 'Escape') {
        e.preventDefault()
        dismissed = open ? query : emojiOpen ? ':' + emojiQ : '/' + cmdQ
        return
      }
    }
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      const t = text.trim()
      if (!t) return
      onSend(t)
      box.setText({ text: '', caret: 0 }, true, false)
      box.forget()
      lastTyping = null
      return
    }
    if (e.key === 'Escape' && replyingTo) {
      e.preventDefault() // taken: a panel around the box doesn't close too
      onCancelReply()
    }
  }

  function input(now: number) {
    syncCaret()
    dismissed = null
    if (text.trim() && typingThrottle(lastTyping, now)) {
      lastTyping = now
      onTyping()
    }
  }

  /** Open the wheel popover (`/wheel` with fewer than two options), starting a quick wheel with `prefill`. */
  export function openWheel(prefill: string[] = []) {
    wheelPrefill = prefill
    wheeling = true
  }

  /** Focus the box: the caret where it was, or (`atEnd`) after all of the text. */
  export function focus(atEnd = false) {
    box?.focus(atEnd ? text.length : undefined)
  }
</script>

<div class="wrap" class:has-wheel={!!wheelChannel}>
  {#if open}
    <div class="mentions menu-surface pop-in" role="listbox" aria-label="Mention someone">
      {#each options as o, i (o.id)}
        <button class="menu-item" role="option" aria-selected={i === pick} class:hi={i === pick}
          onmousedown={(e) => { e.preventDefault(); void choose(o.username) }}><span>@<Name userId={o.id} {serverId} text={o.name} interactive={false} surface="menu" /></span></button>
      {/each}
    </div>
  {:else if emojiOpen}
    <div class="mentions menu-surface pop-in" role="listbox" aria-label="Insert emoji">
      {#each emojiOptions as o, i ('u' in o ? o.u : o.id)}
        <button class="menu-item" role="option" aria-selected={i === pick} class:hi={i === pick}
          onmousedown={(e) => { e.preventDefault(); void chooseEmoji(o) }}>
          {#if 'u' in o}
            <img src={twemojiUrl(withSkin(o, tone))} alt="" draggable="false" /><span>:{o.n}:</span>
          {:else}
            <img src={stillSrc(o)} alt="" draggable="false" {@attach animatedEmoji(o)} /><span>:{o.name}:</span>
            <small>{serverName(o.server_id)}</small>
          {/if}
        </button>
      {/each}
    </div>
  {:else if cmdOpen}
    <div class="mentions menu-surface pop-in" role="listbox" aria-label="Commands">
      {#each cmdOptions as c, i (c.name)}
        <button class="menu-item" role="option" aria-selected={i === pick} class:hi={i === pick}
          onmousedown={(e) => { e.preventDefault(); void chooseCommand(c) }}>
          <span>{@render usage(c)}</span><small>{c.description}</small>
        </button>
      {/each}
    </div>
  {:else if typing}
    <div class="mentions command-tip menu-surface pop-in" role="status">
      <span>{@render usage(typing)}<span class="arg">{` · ${typing.hint}`}</span></span>
    </div>
  {/if}
  {#snippet usage(c: SlashCommand)}
    {@const name = `/${c.name}`}
    {name}{#if c.usage.startsWith(name)}<span class="arg">{c.usage.slice(name.length)}</span>{:else}<span class="arg">{` ${c.usage}`}</span>{/if}
  {/snippet}
  {#if replyingTo}
    <div class="replying">Replying to <strong class="name-clip">{replyingTo}</strong>
      <button class="icon-btn icon-btn-xs" aria-label="Cancel reply" onclick={onCancelReply}><Icon name="x" size={14} /></button></div>
  {/if}
  <EmojiBox bind:this={box} bind:text class="composer-input" label={placeholder} {placeholder} mention={mentionName}
    onkeydown={key} oninput={input} onclick={syncCaret} oncaret={(at) => (caret = at)}
    onkeyup={(e) => { if (e.key.startsWith('Arrow') && !shown) syncCaret() }} />
  <button bind:this={emojiBtn} class="emoji-btn" type="button" aria-label="Emoji" data-menu-toggle onclick={() => (picking = !picking)}><Icon name="smile" size={20} /></button>
  {#if picking && emojiBtn}
    <EmojiPopover anchor={emojiBtn} onPick={pickEmoji} onClose={() => (picking = false)} />
  {/if}
  {#if wheelChannel}
    <button bind:this={wheelBtn} class="emoji-btn wheel-btn" class:on={wheeling} type="button" aria-label="Picker wheel" data-tip="Picker wheel" data-menu-toggle
      aria-haspopup="dialog" aria-expanded={wheeling} onclick={() => { wheelPrefill = null; wheeling = !wheeling }}><Icon name="wheel" size={20} /></button>
    {#if wheeling && wheelBtn}
      {#key wheelPrefill}
        <WheelPopover anchor={wheelBtn} channelId={wheelChannel} {serverId} prefill={wheelPrefill} onClose={() => (wheeling = false)} />
      {/key}
    {/if}
  {/if}
</div>

<style>
  .wrap { position: relative; margin: 4px 18px 16px; background: var(--bg-3); border-radius: var(--radius-lg); }
  /* focus: a faint accent edge, inset so the box doesn't move (the EmojiBox itself has no outline) */
  .wrap:focus-within { box-shadow: inset 0 0 0 1px var(--focus-soft); }
  .mentions { position: absolute; left: 0; bottom: calc(100% + 6px); min-width: 200px; z-index: 5; --pop-origin: bottom left; }
  .mentions .menu-item { font-size: 14px; }
  .mentions img { width: 20px; height: 20px; }
  .mentions small { margin-left: auto; padding-left: 12px; color: var(--text-3); font-size: 11px; }
  /* slash commands (#262): a command's arguments (and the tip's key hint) quieter than its name */
  .mentions .arg { color: var(--text-3); }
  /* the usage tip: the list's surface and placement with one row's padding, not something to press */
  .command-tip { padding: 6px 10px; font-size: 13px; pointer-events: none; }
  .emoji-btn { position: absolute; right: 8px; bottom: 8px; width: 32px; height: 32px; border: 0; border-radius: var(--radius-sm); background: transparent; color: var(--text-3); display: grid; place-items: center; }
  /* one step up from the box's bg-3, as every icon button's hover */
  .emoji-btn:hover { background: var(--bg-4); color: var(--text); }
  /* the picker wheel's button (#158), left of the emoji button */
  .wheel-btn { right: 44px; }
  .wheel-btn.on { background: var(--bg-4); color: var(--text); }
  .replying { display: flex; align-items: center; gap: 6px; padding: 8px 14px 0; font-size: 12px; color: var(--text-3); }
  .replying strong { color: var(--text-2); }
  .replying button { margin-left: auto; }
  /* the composer is bg-3: one step up from it */
  .replying button:hover { background: var(--bg-4); }
  /* grows with what's in it (images included), up to 200px, then scrolls */
  .wrap :global(.composer-input) { min-height: 48px; max-height: 200px; padding: 14px 48px 14px 14px; }
  .wrap.has-wheel :global(.composer-input) { padding-right: 84px; }
</style>
