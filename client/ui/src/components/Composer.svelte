<script lang="ts" module>
  import { SvelteSet } from 'svelte/reactivity'
  import type { StagedFile } from '../lib/tauri'
  // Unsent text per channel, kept while the app runs (switching channels doesn't lose it).
  const drafts = new Map<string, string>()
  // Spotify links the "Share to the stage?" chip (#288) has offered and won't again this session: shared, dismissed,
  // or sent as a message. By lib/stageShare's key, so another copy of the same link counts too.
  const nudged = new SvelteSet<string>()
  /** Tests: as after a restart. */
  export function forgetStageNudges(): void {
    nudged.clear()
  }
  // Files queued per channel (#57), kept the same way.
  const fileDrafts = new Map<string, StagedFile[]>()
</script>

<script lang="ts">
  import { untrack } from 'svelte'
  import Icon from './Icon.svelte'
  import EmojiBox, { type Snap } from './EmojiBox.svelte'
  import EmojiPopover from './EmojiPopover.svelte'
  import GifPopover from './GifPopover.svelte'
  import WheelPopover from './WheelPopover.svelte'
  import PollComposer from './PollComposer.svelte'
  import { customToken, savedTone, search, twemojiUrl, withSkin, type Std } from '../lib/emoji'
  import { applyEmoji, emojiQuery, insertAt } from '../lib/emojiComplete'
  import { animatedEmoji, stillSrc } from '../lib/animate.svelte'
  import { typingThrottle } from '../lib/markdown'
  import { applyMention, mentionQuery, suggest, type Mentionable } from '../lib/mentions'
  import { applyCommand, commandQuery, suggestCommands, typingCommand, type SlashCommand } from '../lib/commands'
  import type { CustomEmoji } from '../lib/protocol/CustomEmoji'
  import type { Gif } from '../lib/protocol/Gif'
  import { memberServers } from '../lib/reactions'
  import { app } from '../lib/store.svelte'
  import { displayName } from '../lib/selectors'
  import Name from './Name.svelte'
  import SpotifyIcon from './SpotifyIcon.svelte'
  import { spotifyLinkIn } from '../lib/stageShare'
  import ComposerFiles from './ComposerFiles.svelte'
  import { api, errorText, type StageResult } from '../lib/tauri'
  import { MAX_BYTES, MAX_FILES, TOO_BIG, TOO_MANY } from '../lib/attachments'

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
    pollChannel = null,
    commands = [],
    onSendGif,
    stageNudge = null,
    files: filesHere = false,
  }: {
    placeholder: string
    draftKey: string
    replyingTo: string | null
    onCancelReply: () => void
    /** `files`: what was queued with it (#57); empty for a plain message. */
    onSend: (text: string, files: StagedFile[]) => void
    onTyping: () => void
    /** People who can be @mentioned here (autocomplete). */
    mentionables?: Mentionable[]
    /** The server it posts in (null in DMs and groups): suggested names use its role colours. */
    serverId?: string | null
    /** The chat a picker wheel spins in (#158): shows the wheel button. None where wheels don't spin (threads). */
    wheelChannel?: string | null
    /** The chat a poll posts in (#157): shows the poll button. None where polls don't go (threads). */
    pollChannel?: string | null
    /** Slash commands that run here (#262, lib/commands.ts): suggested on `/`, with a usage tip. None where they don't. */
    commands?: readonly SlashCommand[]
    /** Sends a GIF picked in the GIF picker (#58) at once (`link`: its GIPHY page, the message's text): shows the GIF button. */
    onSendGif?: (gif: Gif, link: string) => void
    /** A voice channel's chat while you're in its call (#288): a Spotify link in the box offers "Share to the stage?".
     *  `share` puts a link on the stage; `up` is what's on it now (not offered again). None anywhere else. */
    stageNudge?: { share: (url: string) => Promise<void>; up: readonly string[] } | null
    /** Files can be sent here (#57): shows the attach button and takes pasted files. */
    files?: boolean
  } = $props()

  // The composer remounts per channel ({#key} in Shell), so reading the key once is intended.
  let text = $state(untrack(() => drafts.get(draftKey) ?? ''))
  $effect(() => {
    if (text) drafts.set(draftKey, text)
    else drafts.delete(draftKey)
  })
  let lastTyping: number | null = null

  // Files queued to go with the next message (#57).
  let files = $state<StagedFile[]>(untrack(() => fileDrafts.get(draftKey) ?? []))
  $effect(() => {
    if (files.length) fileDrafts.set(draftKey, files)
    else fileDrafts.delete(draftKey)
  })
  let fileError = $state<string | null>(null)

  function addStaged(results: StageResult[]) {
    fileError = null
    for (const r of results) {
      if (r.status === 'refused') {
        fileError = r.error
      } else if (files.length >= MAX_FILES) {
        fileError = TOO_MANY
        void api.unstageAttachment(r.file.id)
      } else if (r.file.size > MAX_BYTES) {
        fileError = TOO_BIG
        void api.unstageAttachment(r.file.id)
      } else {
        files = [...files, r.file]
      }
    }
  }

  /** Queue files by path (dropped on the chat, or picked). */
  export async function addPaths(paths: string[]) {
    if (!filesHere || !paths.length) return
    if (files.length >= MAX_FILES) {
      fileError = TOO_MANY
      return
    }
    try {
      addStaged(await api.stageAttachmentPaths(paths))
    } catch (e) {
      fileError = errorText(e)
    }
  }

  async function pickFiles() {
    const paths = await api.pickAttachmentFiles().catch(() => [] as string[])
    await addPaths(paths)
    box?.focus()
  }

  // A screenshot (or a file copied in a file manager) pasted into the box: queued, not typed.
  async function pasteFiles(e: ClipboardEvent) {
    const list = Array.from(e.clipboardData?.files ?? [])
    if (!filesHere || !list.length) return
    e.preventDefault()
    e.stopPropagation()
    for (const f of list) {
      if (files.length >= MAX_FILES) {
        fileError = TOO_MANY
        return
      }
      try {
        const file = await api.stageAttachmentBytes(f.name || 'pasted', new Uint8Array(await f.arrayBuffer()))
        addStaged([{ status: 'staged', file }])
      } catch (err) {
        fileError = errorText(err)
      }
    }
  }

  function removeFile(id: string) {
    files = files.filter((f) => f.id !== id)
    fileError = null
    void api.unstageAttachment(id)
  }
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
  // "Share to the stage?" (#288): the first Spotify link in the box, unless it's on the stage already or was offered
  // before; never over a list or the usage tip (they're about what's typed right now).
  const nudgeLink = $derived.by(() => {
    if (!stageNudge) return null
    const l = spotifyLinkIn(text)
    if (!l || nudged.has(l.key) || stageNudge.up.some((u) => spotifyLinkIn(u)?.key === l.key)) return null
    return l
  })
  const nudgeOpen = $derived(!!nudgeLink && !shown && !typing)
  let nudgeBusy = $state(false)
  let nudgeError = $state('')
  let nudgeErrorFor: string | null = null
  $effect(() => {
    // an error is about the link it was for
    if (nudgeLink?.key !== nudgeErrorFor) nudgeError = ''
  })
  async function shareNudged() {
    const l = nudgeLink
    if (!l || !stageNudge || nudgeBusy) return
    nudgeBusy = true
    nudgeError = ''
    try {
      await stageNudge.share(l.url)
      nudged.add(l.key)
      // the share posts the link in the chat itself (#288): a box holding only that link would post it twice
      if (text.trim() === l.url) {
        box.setText({ text: '', caret: 0 }, true, false)
        box.forget()
      }
    } catch (e) {
      nudgeErrorFor = l.key
      nudgeError = errorText(e)
    } finally {
      nudgeBusy = false
    }
  }
  const serverName = (id: string) => app.state.servers.find((s) => s.id === id)?.name ?? ''
  const emojiText = (e: Std | CustomEmoji) => ('u' in e ? withSkin(e, tone) : customToken(e))

  let picking = $state(false)
  let emojiBtn: HTMLButtonElement | undefined = $state()
  // the GIF picker (#58)
  let gifPicking = $state(false)
  let gifBtn: HTMLButtonElement | undefined = $state()
  function pickGif(gif: Gif, link: string) {
    gifPicking = false
    onSendGif?.(gif, link)
    box?.focus()
  }
  // the wheel popover (#158); `wheelPrefill`: options typed after `/wheel` (fewer than two)
  let wheeling = $state(false)
  let wheelPrefill = $state<string[] | null>(null)
  let wheelBtn: HTMLButtonElement | undefined = $state()
  // the poll composer (#157); `pollPrefill`: what `/poll` was given
  let polling = $state(false)
  let pollPrefill = $state<{ question: string; options: string[] } | null>(null)
  let pollBtn: HTMLButtonElement | undefined = $state()

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
        dismiss()
        return
      }
    }
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      const t = text.trim()
      if (!t && !files.length) return
      // a link sent as it is has been seen: the chip doesn't come back for it
      const sent = spotifyLinkIn(t)
      if (sent && stageNudge) nudged.add(sent.key)
      onSend(t, files)
      files = []
      fileError = null
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

  // Esc, a press outside the composer or focus leaving it (#264) closes the list showing, until its token changes
  function dismiss() {
    if (shown) dismissed = open ? query : emojiOpen ? ':' + emojiQ : '/' + cmdQ
  }
  let wrap: HTMLDivElement | undefined = $state()
  $effect(() => {
    if (!shown) return
    const down = (e: PointerEvent) => {
      if (!wrap?.contains(e.target as Node)) dismiss()
    }
    window.addEventListener('pointerdown', down, true)
    return () => window.removeEventListener('pointerdown', down, true)
  })
  function focusOut(e: FocusEvent) {
    // the window losing focus (another app) isn't leaving the composer
    if (!document.hasFocus()) return
    if (!wrap?.contains(e.relatedTarget as Node | null)) dismiss()
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
    polling = false
    wheelPrefill = prefill
    wheeling = true
  }

  /** Open the poll composer (`/poll`), filled with what was typed after it. */
  export function openPoll(prefill: { question: string; options: string[] }) {
    wheeling = false
    pollPrefill = prefill
    polling = true
  }

  /** Focus the box: the caret where it was, or (`atEnd`) after all of the text. */
  export function focus(atEnd = false) {
    box?.focus(atEnd ? text.length : undefined)
  }
</script>

<div class="wrap" class:has-gif={!!onSendGif} class:has-wheel={!!wheelChannel} class:has-poll={!!pollChannel} class:has-files={filesHere} bind:this={wrap} onfocusout={focusOut}
  onpastecapture={pasteFiles}>
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
  {:else if nudgeOpen && nudgeLink}
    <div class="mentions stage-nudge menu-surface pop-in" role="group" aria-label="Share to the stage">
      <div class="nudge-row">
        <SpotifyIcon />
        <button type="button" class="btn btn-quiet nudge-share" disabled={nudgeBusy} data-tip="Show it on the stage for everyone in the call"
          onmousedown={(e) => e.preventDefault()} onclick={() => void shareNudged()}>Share to the stage?</button>
        <button type="button" class="icon-btn icon-btn-xs" aria-label="Dismiss" data-tip="Not this one"
          onmousedown={(e) => e.preventDefault()} onclick={() => nudged.add(nudgeLink.key)}><Icon name="x" size={14} /></button>
      </div>
      {#if nudgeError}<p class="nudge-error" role="alert">{nudgeError}</p>{/if}
    </div>
  {/if}
  {#snippet usage(c: SlashCommand)}
    {@const name = `/${c.name}`}
    {name}{#if c.usage.startsWith(name)}<span class="arg">{c.usage.slice(name.length)}</span>{:else}<span class="arg">{` ${c.usage}`}</span>{/if}
  {/snippet}
  <ComposerFiles {files} onRemove={removeFile} error={fileError} />
  {#if replyingTo}
    <div class="replying">Replying to <strong class="name-clip">{replyingTo}</strong>
      <button class="icon-btn icon-btn-xs" aria-label="Cancel reply" onclick={onCancelReply}><Icon name="x" size={14} /></button></div>
  {/if}
  <EmojiBox bind:this={box} bind:text class="composer-input" label={placeholder} {placeholder} mention={mentionName}
    onkeydown={key} oninput={input} onclick={syncCaret} oncaret={(at) => (caret = at)}
    onkeyup={(e) => { if (e.key.startsWith('Arrow') && !shown) syncCaret() }} />
  {#if filesHere}
    <button class="emoji-btn attach-btn" type="button" aria-label="Attach files" data-tip="Attach files" onclick={pickFiles}><Icon name="paperclip" size={20} /></button>
  {/if}
  <button bind:this={emojiBtn} class="emoji-btn" type="button" aria-label="Emoji" data-menu-toggle onclick={() => (picking = !picking)}><Icon name="smile" size={20} /></button>
  {#if picking && emojiBtn}
    <EmojiPopover anchor={emojiBtn} onPick={pickEmoji} onClose={() => (picking = false)} />
  {/if}
  {#if onSendGif}
    <button bind:this={gifBtn} class="emoji-btn gif-btn" class:on={gifPicking} type="button" aria-label="GIF" data-tip="GIF" data-menu-toggle
      aria-haspopup="dialog" aria-expanded={gifPicking} onclick={() => (gifPicking = !gifPicking)}><Icon name="gif" size={20} /></button>
    {#if gifPicking && gifBtn}
      <GifPopover anchor={gifBtn} onPick={pickGif} onClose={() => (gifPicking = false)} />
    {/if}
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
  {#if pollChannel}
    <button bind:this={pollBtn} class="emoji-btn poll-btn" class:on={polling} type="button" aria-label="Poll" data-tip="Poll" data-menu-toggle
      aria-haspopup="dialog" aria-expanded={polling} onclick={() => { pollPrefill = null; polling = !polling }}><Icon name="poll" size={20} /></button>
    {#if polling && pollBtn}
      {#key pollPrefill}
        <PollComposer anchor={pollBtn} channelId={pollChannel} prefill={pollPrefill} onClose={() => (polling = false)} />
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
  /* Share to the stage? (#288): the tip's surface and place, with something to press */
  .stage-nudge { min-width: 0; padding: 4px 4px 4px 10px; gap: 4px; color: var(--text); }
  .nudge-row { display: flex; align-items: center; gap: 6px; }
  .nudge-share { height: 28px; padding: 0 10px; font-size: 13px; white-space: nowrap; }
  .nudge-error { margin: 0 6px 4px 0; color: var(--danger-text); font-size: 12px; }
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
  /* the poll button (#157), left of the wheel button */
  .poll-btn { right: 80px; }
  .poll-btn.on { background: var(--bg-4); color: var(--text); }
  .wrap.has-poll :global(.composer-input) { padding-right: 120px; }
  /* the GIF button (#58), right next to the emoji button: the wheel and poll buttons move one place left */
  .gif-btn { right: 44px; }
  .gif-btn.on { background: var(--bg-4); color: var(--text); }
  .wrap.has-gif .wheel-btn { right: 80px; }
  .wrap.has-gif .poll-btn { right: 116px; }
  .wrap.has-gif :global(.composer-input) { padding-right: 84px; }
  .wrap.has-gif.has-wheel :global(.composer-input) { padding-right: 120px; }
  .wrap.has-gif.has-poll :global(.composer-input) { padding-right: 156px; }
  /* attach files (#57): on the left, as in most chat apps */
  .attach-btn { left: 8px; right: auto; }
  .wrap.has-files :global(.composer-input) { padding-left: 48px; }
</style>
