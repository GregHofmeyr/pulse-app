<script lang="ts" module>
  export { INLINE_ERROR_MS } from '../lib/inlineError.svelte'
</script>

<script lang="ts">
  import { actionBarFocus } from '../lib/focus'
  import Icon from './Icon.svelte'
  import DeleteButton from './DeleteButton.svelte'
  import EmojiBox from './EmojiBox.svelte'
  import EmojiPopover from './EmojiPopover.svelte'
  import ReactionRow from './ReactionRow.svelte'
  import LinkEmbed from './LinkEmbed.svelte'
  import GifView from './GifView.svelte'
  import { gifIdOf, gifOnly, gifsOf } from '../lib/giphy'
  import MessageFiles from './MessageFiles.svelte'
  import ComposerFiles from './ComposerFiles.svelte'
  import { shownCaption } from '../lib/attachments'
  import { attachmentSends } from '../lib/attachmentSends.svelte'
  import { previews } from '../lib/linkPreviews.svelte'
  import { reactionKey, type Std } from '../lib/emoji'
  import { pickAdds, react } from '../lib/reactions'
  import { untrack } from 'svelte'
  import { app, clock, openDm, ui } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import { InlineError } from '../lib/inlineError.svelte'
  import type { CustomEmoji } from '../lib/protocol/CustomEmoji'
  import { renderMessage } from '../lib/markdown'
  import { animateEmojiIn } from '../lib/animate.svelte'
  import Avatar from './Avatar.svelte'
  import Name from './Name.svelte'
  import WheelLine from './WheelLine.svelte'
  import PollCard from './PollCard.svelte'
  import { endedSummary, pollOver, shownText } from '../lib/poll'
  import { closePoll } from '../lib/polls.svelte'
  import { mentionLookup, mentionName, showMentions, styleMentions } from '../lib/mentionStyles'
  import { nameCss, resolveNameStyle } from '../lib/nameStyle'
  import { displayName, roleColorFor, userOf } from '../lib/selectors'
  import { callLineText } from '../lib/calls'
  import { appearance } from '../lib/appearance.svelte'
  import { openCard } from '../lib/profileCard.svelte'
  import MemberMenu from './MemberMenu.svelte'
  import MessageMenu from './MessageMenu.svelte'
  import { quoteRequest, quotesChannelsFor } from '../lib/quoteFromMessage'
  import { livePin, pinFor, pinTip, takesPins } from '../lib/pins'
  import { refreshPins } from '../lib/pinSync.svelte'
  import { serverNow } from '../lib/serverClock'
  import { jumpTo } from '../lib/jump'
  import { menuPoint } from '../lib/participantMenu'
  import { nameOpts } from '../lib/theme.svelte'
  import type { Attachment } from 'svelte/attachments'
  import type { Message } from '../lib/protocol/Message'
  import type { Pending } from '../lib/state'
  import { sharePhrase } from '../lib/stageShare'

  let {
    message,
    serverId,
    name,
    nameOf,
    replyTo,
    grouped,
    mine,
    onReply,
    onEdit,
    onDelete,
    mentionNames = [],
    meId = '',
    pending,
    onRetry = () => {},
    onDiscard = () => {},
    onEditClosed = () => {},
  }: {
    message: Message
    /** The server the message is in (null in DMs and groups): names use its nicknames and role colours. */
    serverId: string | null
    name: string
    nameOf: (userId: string | null) => string
    replyTo: Message | null
    grouped: boolean
    mine: boolean
    onReply: () => void
    onEdit: (content: string) => Promise<void>
    onDelete: () => Promise<void>
    /** Usernames that may be mentioned here (rendered as pills). */
    mentionNames?: string[]
    meId?: string
    /** Set when `message` is an unsent one (`pendingMessage`): faded, no actions or reactions, status line under it. */
    pending?: Pending
    onRetry?: () => void
    onDiscard?: () => void
    /** The edit box closed (saved or cancelled): its focus has nowhere to go, so the caller gives it some. */
    onEditClosed?: () => void
  } = $props()
  const uid = $props.id()

  // --- names and avatars: the avatar animates while the pointer is over the row; pills take the person's style ---
  let rowHover = $state(false)
  // Pills restyle only when the HTML or a mentioned person's style changes, not on every state update:
  // the lookup is rebuilt only when people or members change, and the styles are compared as one string.
  const lookup = $derived(mentionLookup(app.state))
  const mentioned = $derived.by(() => {
    const c = message.content.toLowerCase()
    if (!c.includes('@')) return [] as string[]
    const ids = mentionNames.filter((n) => c.includes('@' + n.toLowerCase())).map(lookup)
    return [...new Set(ids.filter((id): id is string => !!id))]
  })
  const cssFor = (id: string) => nameCss(resolveNameStyle(userOf(app.state, id), { roleColor: roleColorFor(app.state, serverId, id), showStyles: appearance.showNameStyles, ...nameOpts('chat') }))
  // with the name each pill shows (#47): a rename relabels it, as a style change restyles it
  const mentionCss = $derived(mentioned.map((id) => `${id}\t${cssFor(id)}\t${displayName(app.state, serverId, id)}`).join('\n'))
  // Only the emoji slice, so the {@html} below re-renders on an emoji change, not on every state update.
  const customEmoji = $derived(app.state.customEmoji)
  const mentionsIn = (_html: string, look: (name: string) => string | null, cssKey: string): Attachment<HTMLElement> => (node) => {
    if (!cssKey || !node.querySelector('.mention')) return
    const rows = new Map(cssKey.split('\n').map((l) => { const [id, css, name] = l.split('\t'); return [id, { css, name }] as const }))
    styleMentions(node, (n) => { const id = look(n); return id && rows.has(id) ? id : null }, (id) => rows.get(id)!.css, (id) => rows.get(id)!.name)
  }
  // the edit box shows mentions as chips with the name that applies, as the composer does
  const editMention = (username: string) => { const id = lookup(username); return id ? displayName(app.state, serverId, id) : null }
  function openMention(e: MouseEvent | KeyboardEvent) {
    if (e instanceof KeyboardEvent && e.key !== 'Enter' && e.key !== ' ') return
    const pill = (e.target as Element).closest('.mention')
    const id = pill && lookup(mentionName(pill))
    if (!pill || !id) return
    if (e instanceof KeyboardEvent) e.preventDefault() // Space mustn't scroll
    openCard(id, serverId, pill)
  }

  let focusIn = $state(false)
  let picking = $state(false)
  let smileBtn: HTMLButtonElement | undefined = $state()
  let editing = $state(false)
  let draft = $state('')
  let editError = $state('')
  const inlineError = new InlineError()
  const showError = inlineError.show
  $effect(() => inlineError.stop)

  // --- the author's menu (#137): right-click (or ContextMenu / Shift+F10 on) the author's name or avatar: View
  // profile, Message, the nickname items. In a DM or group too (#189), with no server nickname; Message only in a
  // group (a DM is already that), and in a DM Mute conversation, as the DM list's menu. ---
  let authorMenu = $state<{ x: number; y: number; opener: HTMLElement } | null>(null)
  const authorMenuOn = $derived(!!message.author_id && !pending)
  const channelKind = $derived(app.state.channels[message.channel_id]?.kind)
  /** `zone`: the avatar or the name line; the menu returns focus to its button. */
  function openAuthorMenu(e: MouseEvent | KeyboardEvent, zone: HTMLElement) {
    if (!authorMenuOn) return
    const opener = zone.querySelector<HTMLElement>('button') ?? zone
    e.preventDefault()
    authorMenu = { ...menuPoint(e, opener), opener }
  }
  function authorKey(e: KeyboardEvent) {
    if (e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey)) openAuthorMenu(e, e.currentTarget as HTMLElement)
  }
  // --- the message's own menu (#178): Pin message (a flyout of lengths) or Unpin message. From the action bar's More,
  // or a right-click on the message (the author's name and avatar keep their person menu). Not in quotes or clips
  // channels (QuoteItem and the clip views are theirs), nor on an unsent or deleted message. ---
  let msgMenu = $state<{ x: number; y: number; opener: HTMLElement } | null>(null)
  const pinnable = $derived(takesPins(channelKind))
  const msgMenuOn = $derived(pinnable && !pending && !message.deleted && !editing)
  function openMsgMenu(e: MouseEvent, opener: HTMLElement) {
    // the author's own menu took it (their name or avatar), or there's nothing to show
    if (e.defaultPrevented || !msgMenuOn) return
    // a link, an image or text you selected keeps the system menu (Copy, Copy link)
    if (e.type === 'contextmenu') {
      const sel = getSelection()
      if ((e.target as Element).closest?.('a, img, video, input, textarea, [contenteditable]')) return
      if (sel && !sel.isCollapsed && sel.anchorNode && (opener.contains(sel.anchorNode) || opener.contains(sel.focusNode))) return
    }
    e.preventDefault()
    msgMenu = { ...menuPoint(e, opener), opener }
  }
  // the clock moves every 30 s: a timed pin's time left (and its end) follow it, before the sweep's event. By the
  // server's clock (#258): one running ahead here would otherwise end a pin the server still holds.
  const now = $derived((void clock.nowIso, serverNow()))
  const pin = $derived(livePin(message, now))
  const pinMark = $derived(pin ? pinTip(nameOf(pin.pinned_by), pin.expires_at, now) : '')
  // A refused pin or unpin means the pin here was out of date: show why, and fetch the chat's pins to set it right.
  const report = (p: Promise<unknown>) =>
    p.catch((e) => {
      showError(errorText(e))
      void refreshPins(message.channel_id)
    })
  // your own poll, still open, can be ended from the message's menu (#157); a refusal fetches the poll afresh
  const endable = $derived(!!message.poll && mine && !pollOver(message.poll, now))
  const endPoll = () => closePoll(message).catch((e) => showError(errorText(e)))
  // Add to quotes (#316): the message as a quote, its author speaking, in one of its server's quotes channels (none in
  // a DM or group: no item there). One request id per message, so a double pick posts it once.
  const quotesChannels = $derived(msgMenu ? quotesChannelsFor(app.state, message).map((c) => ({ id: c.id, name: c.name ?? '' })) : [])
  let quoteNonce: string | null = null
  function addToQuotes(channelId: string) {
    const server = app.state.channels[message.channel_id]?.server_id
    if (!server) return
    quoteNonce ??= crypto.randomUUID()
    const name = app.state.channels[channelId]?.name ?? 'quotes'
    return api.postQuote(channelId, quoteRequest(app.state, message, server, quoteNonce)).then(
      () => (ui.notice = `Added to #${name}`),
      (e) => showError(errorText(e)),
    )
  }
  function messageAuthor(userId: string) {
    openDm(userId).then((channelId) => (ui.goTo = channelId), (e) => showError(errorText(e)))
  }
  async function del() {
    try {
      await onDelete()
    } catch (e) {
      showError(errorText(e))
    }
  }
  async function hideEmbeds() {
    try {
      await api.hideEmbed(message.id, null)
    } catch (e) {
      showError(errorText(e))
    }
  }
  // a GIPHY link shows as the GIF (#58), never as a preview card (an older server may have made one)
  const embeds = $derived(!pending && !message.deleted && previews.show ? (message.embeds ?? []).filter((e) => !gifIdOf(e.url)) : [])
  // GIFs (#58): a picked one, or the GIPHY links in the text; a message that is only the link shows only the GIF
  // a pasted link's GIF is a link preview: it follows "Show link previews" (a picked GIF always shows); one GIPHY
  // couldn't give falls back to the link
  let failedGifs = $state<string[]>([])
  const gifs = $derived(message.gif || previews.show ? gifsOf(message).filter((g) => !failedGifs.includes(g.id)) : [])
  const onlyGif = $derived(gifs.length > 0 && gifOnly(message))
  const time = $derived(new Date(message.created_at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }))

  let editBox: EmojiBox | undefined = $state()
  function startEdit() {
    draft = shownCaption(message)
    editError = ''
    editing = true
  }
  // the box opens focused, the caret after the text
  $effect(() => {
    if (editBox) untrack(() => editBox!.focus(draft.length))
  })
  // a second Enter while the first save is in flight must not send the edit twice
  let saving = false
  async function save() {
    if (saving) return
    const d = draft.trim()
    editError = ''
    // a message with files may lose its caption (#57): they're the message then
    if ((d || message.attachments?.length) && d !== shownCaption(message)) {
      saving = true
      try {
        await onEdit(d)
      } catch (e) {
        editError = errorText(e) // the box stays open with the text intact
        return
      } finally {
        saving = false
      }
    }
    closeEdit()
  }
  function closeEdit() {
    // only focus the box had (or nobody has): a save landing after the person moved on leaves
    // their focus where it is
    const handBack = !!editBox?.hasFocus() || document.activeElement === document.body || !document.activeElement
    editing = false
    editError = ''
    if (handBack) onEditClosed()
  }
  function pick(e: Std | CustomEmoji, { keep }: { keep: boolean }) {
    if (!keep) picking = false
    const key = reactionKey(e)
    void react(() => app.state, (s) => (app.state = s), message.channel_id, message.id, key, pickAdds(message.reactions, key, keep), meId, showError)
  }
  // after the edit box's own keys (IME, undo/redo); Shift+Enter, not taken here, adds a line
  function key(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      void save()
    } else if (e.key === 'Escape') {
      e.preventDefault()
      // a save in flight lands anyway: the box stays until it has (or shows why not)
      if (!saving) closeEdit()
    }
  }
</script>

{#snippet item()}
{#if message.kind === 'system'}
  {#if message.call}
    <!-- a call line (#50) reads per member: "Missed call from sam" for those who never joined -->
    {@const text = callLineText(app.state, message)}
    <div class="system call-line" class:missed={text.startsWith('Missed')} data-message-id={message.id}><Icon name="phone" size={14} /> {text} <span class="time">{time}</span></div>
  {:else if message.pin_line}
    <!-- a pin's line (#178): its message jumps to the pinned one, See pins opens the chat's pins -->
    {@const pl = message.pin_line}
    {@const length = pinFor(pl.duration)}
    <div class="system pin-line" data-message-id={message.id}><Icon name="pin" size={14} /> {nameOf(pl.by)} pinned
      <button type="button" class="link-btn" onclick={() => void jumpTo(message.channel_id, pl.message_id)}>a message</button>{length ? ` ${length}` : ''} ·
      <button type="button" class="link-btn" onclick={() => (ui.openPins = message.channel_id)}>See pins</button> <span class="time">{time}</span></div>
  {:else if message.wheel}
    <!-- a picker wheel spin (#158): spins live as it arrives, then the line -->
    <WheelLine {message} {serverId} {time} />
  {:else if message.poll_ended}
    <!-- a poll's ended line (#157): See poll jumps to the poll -->
    {@const pe = message.poll_ended}
    <div class="system pin-line" data-message-id={message.id}><Icon name="poll" size={14} /> {nameOf(pe.author_id)}'s poll ended{endedSummary(pe.winners, pe.votes)} ·
      <button type="button" class="link-btn" onclick={() => void jumpTo(message.channel_id, pe.poll_message_id)}>See poll</button> <span class="time">{time}</span></div>
  {:else if message.stage_share_line}
    <!-- a share to the voice stage (#234), with today's names: "🎵 alex shared a Jam to the stage" -->
    {@const sl = message.stage_share_line}
    <div class="system" data-message-id={message.id}>🎵 {nameOf(sl.by)} shared {sharePhrase(sl.kind)} to the stage <span class="time">{time}</span></div>
  {:else}
  <div class="system" data-message-id={message.id}>{message.content} <span class="time">{time}</span></div>
  {/if}
{:else}
  <!-- focus anywhere inside shows the action bar (hover alone left keyboard users without it) -->
  <div class="msg" data-message-id={message.id} {...actionBarFocus((v) => (focusIn = v))} class:grouped class:mentioned={!!meId && message.mentions.includes(meId)}
    onpointerenter={() => (rowHover = true)} onpointerleave={() => (rowHover = false)} oncontextmenu={(e) => openMsgMenu(e, e.currentTarget)}>
    {#if replyTo}
      <div class="reply"><span class="hook"></span><strong>{#if replyTo.author_id}<Name userId={replyTo.author_id} {serverId} text={nameOf(replyTo.author_id)} />{:else}{nameOf(null)}{/if}</strong>
        <span class="snippet">{replyTo.deleted ? 'original message deleted' : showMentions(shownText(replyTo), lookup, (id) => displayName(app.state, serverId, id)).slice(0, 120)}</span></div>
    {:else if message.reply_to_id}
      <div class="reply"><span class="hook"></span><span class="snippet">original message not loaded</span></div>
    {/if}
    <div class="row">
      {#if grouped && !replyTo}
        <span class="gutter">{#if pin}<span class="pin-mark" data-tip={pinMark}><Icon name="pin" size={14} /></span>{/if}</span>
      {:else}
        <!-- the author menu is a pointer shortcut; the keyboard route is ContextMenu / Shift+F10 on the avatar or name -->
        <span class="author" role="presentation" oncontextmenu={(e) => openAuthorMenu(e, e.currentTarget)} onkeydown={authorKey}><Avatar id={message.author_id ?? ''} {name} size={40} interactive={!!message.author_id} {serverId} hovered={() => rowHover} /></span>
      {/if}
      <div class="body">
        {#if !grouped || replyTo}<div class="meta"><strong role="presentation" oncontextmenu={(e) => openAuthorMenu(e, e.currentTarget)} onkeydown={authorKey}>{#if message.author_id}<Name userId={message.author_id} {serverId} text={name} />{:else}{name}{/if}</strong><span class="time">{time}</span>{#if pin}<span class="pin-mark" data-tip={pinMark}><Icon name="pin" size={14} /></span>{/if}</div>{/if}
        {#if message.deleted}
          <p class="deleted">message deleted</p>
        {:else if editing}
          <EmojiBox bind:this={editBox} bind:text={draft} class="edit" label="Edit message" describedby="{uid}-hint" onkeydown={key} mention={editMention} />
          <small class="hint" id="{uid}-hint">Enter to save · Esc to cancel · Shift+Enter for a new line</small>
          {#if editError}<p class="err" role="alert">{editError}</p>{/if}
        {:else if message.poll}
          <!-- a poll (#157): the card stands in for its text (which is the poll for clients that don't know polls) -->
          <PollCard {message} {nameOf} />
        {:else}
          <!-- renderMessage output is DOMPurify-sanitised (allow-list, cached per message); the only {@html} in the app. -->
          {@const html = renderMessage(message, mentionNames, customEmoji)}
          <!-- a mention pill opens its person's card, by click or (focused: styleMentions makes known pills buttons) Enter or Space -->
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          {#if !message.caption_is_fallback}
            {#if !onlyGif}<div class="content" {@attach animateEmojiIn(html)} {@attach mentionsIn(html, lookup, mentionCss)} onclick={openMention} onkeydown={openMention}>{@html html}</div>{/if}
            {#each gifs as g (g.id)}<GifView gif={g} onError={showError} onFail={() => (failedGifs = [...failedGifs, g.id])} />{/each}
            {#if message.edited_at}<span class="edited">(edited)</span>{/if}
          {/if}
        {/if}
        <!-- files (#57): a sent message's, or the chips and upload bar of one on its way -->
        {#if pending?.files?.length}
          <ComposerFiles files={pending.files} progress={pending.status === 'pending' ? (attachmentSends.fraction(pending.nonce) ?? 0) : null}
            error={pending.status === 'failed' ? attachmentSends.error(pending.nonce) : null} />
        {:else if !pending && !message.deleted && message.attachments?.length}
          <MessageFiles {message} onError={showError} />
        {/if}
        {#each embeds as e (e.url)}<LinkEmbed embed={e} messageId={message.id} channelId={message.channel_id} {mine} onError={showError} />{/each}
        {#if !pending}<ReactionRow {message} {meId} {nameOf} onError={showError} />{/if}
        {#if inlineError.text}<p class="err" role="alert">{inlineError.text}</p>{/if}
      </div>
      {#if !pending && !message.deleted && !editing}
        <div class="actions" class:open={picking || focusIn || !!msgMenu}>
          <button bind:this={smileBtn} aria-label="Add reaction" data-menu-toggle onclick={() => (picking = !picking)}><Icon name="smile" size={16} /></button>
          <button aria-label="Reply" onclick={onReply}><Icon name="reply" size={16} /></button>
          {#if mine}
            {#if !message.poll}<button aria-label="Edit" onclick={startEdit}><Icon name="pencil" size={16} /></button>{/if}
            {#if embeds.length}<button aria-label="Remove previews" onclick={() => void hideEmbeds()}><Icon name="unlink" size={16} /></button>{/if}
            <DeleteButton what={message.thread_root_id ? 'reply' : 'message'} onConfirm={() => void del()} />
          {/if}
          {#if msgMenuOn}
            <button aria-label="More" aria-haspopup="menu" aria-expanded={!!msgMenu} data-menu-toggle
              onclick={(e) => (msgMenu ? (msgMenu = null) : openMsgMenu(e, e.currentTarget))}><Icon name="dots" size={16} /></button>
          {/if}
        </div>
        {#if picking && smileBtn}
          <EmojiPopover anchor={smileBtn} onPick={pick} onClose={() => (picking = false)} />
        {/if}
      {/if}
    </div>
  </div>
{/if}
{/snippet}

{#if pending}
  <div class="pend">
    <div class="unsent" style:opacity={0.6}>{@render item()}</div>
    <div class="status" class:failed={pending.status === 'failed'}>
      {#if pending.status === 'failed'}
        <span class="fail">Failed to send</span>
        <button onclick={onRetry}>Retry</button>
        <button onclick={onDiscard}>Discard</button>
      {:else}
        <span>Sending…</span>
      {/if}
    </div>
  </div>
{:else}
  {@render item()}
{/if}
<!-- outside the row, so focus in the menu doesn't count as focus in the message (its action bar) -->
{#if msgMenu}
  {#key msgMenu}<MessageMenu pinned={!!pin} x={msgMenu.x} y={msgMenu.y} opener={msgMenu.opener}
    onPin={(d) => report(api.pinMessage(message.id, d))} onUnpin={() => report(api.unpinMessage(message.id))}
    onEndPoll={endable ? endPoll : undefined} {quotesChannels} onQuote={addToQuotes} onClose={() => (msgMenu = null)} />{/key}
{/if}
{#if authorMenu && message.author_id}
  {#key authorMenu}<MemberMenu {serverId} userId={message.author_id} x={authorMenu.x} y={authorMenu.y} opener={authorMenu.opener}
    onMessage={channelKind === 'dm' ? undefined : messageAuthor} muteChannelId={channelKind === 'dm' ? message.channel_id : null} onClose={() => (authorMenu = null)} />{/key}
{/if}

<style>
  .author { display: contents; }
  .status { display: flex; gap: 10px; align-items: baseline; padding: 0 18px 4px 72px; font-size: 12px; color: var(--text-3); }
  .status .fail { color: var(--danger-text); }
  .status button { border: 0; background: none; padding: 0; font-size: 12px; color: var(--text-2); text-decoration: underline; }
  /* At narrow widths (#237) more lines run to the right edge, under the quick-action bar: --bar-room grows from 0 at
     1200px to 4px at 1100px and below, a ramp rather than a breakpoint so resizing never makes the list jump. The bar
     moves down by 1.5x it (-10px to -4px, clear of the line above) and grouped rows gain it as top padding. The same in
     QuoteItem. */
  .msg { --bar-room: clamp(0px, calc((1200px - 100vw) / 25), 4px); padding: 8px 18px 2px; position: relative; }
  .msg.grouped { padding-top: calc(1px + var(--bar-room)); }
  .msg:hover { background: var(--hover-tint); }
  .row { display: flex; gap: 14px; }
  .gutter { width: 40px; flex-shrink: 0; }
  .body { flex: 1; min-width: 0; }
  .meta { display: flex; align-items: baseline; gap: 8px; margin-bottom: 2px; }
  .meta strong { font-size: 14.5px; }
  .time { font-size: 11px; color: var(--text-3); }
  .content { color: var(--text-body); line-height: 1.5; overflow-wrap: anywhere; }
  .content :global(p) { margin: 0; }
  .content :global(code) { font-family: 'JetBrains Mono', ui-monospace, monospace; font-size: 12.5px; background: var(--bg-inset); border: 1px solid var(--line); border-radius: var(--radius-xs); padding: 1px 6px; }
  .content :global(pre) { background: var(--bg-inset); border-radius: var(--radius-sm); padding: 10px; overflow-x: auto; }
  .content :global(pre code) { border: 0; padding: 0; }
  .content :global(a) { color: var(--accent-text); }
  .content :global(blockquote) { margin: 4px 0; padding-left: 10px; border-left: 3px solid var(--bg-4); color: var(--text-2); }
  .edited { font-size: 11px; color: var(--text-3); }
  .deleted { margin: 0; font-style: italic; color: var(--text-3); }
  .reply { display: flex; align-items: center; gap: 6px; padding-left: 20px; margin-bottom: 4px; font-size: 13px; color: var(--text-3); min-width: 0; }
  .reply strong { color: var(--text-2); }
  .hook { width: 28px; height: 10px; margin-top: 10px; border-left: 2px solid var(--bg-4); border-top: 2px solid var(--bg-4); border-top-left-radius: 6px; flex-shrink: 0; }
  .snippet { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .actions { position: absolute; top: calc(-10px + 1.5 * var(--bar-room)); right: 18px; display: flex; opacity: 0; pointer-events: none; background: var(--bg-3); border: 1px solid var(--bg-4); border-radius: var(--radius-sm); will-change: opacity; }
  /* fades in, hides at once (#173): sweeping down a channel never leaves half-faded bars behind */
  .msg:hover .actions, .actions.open { opacity: 1; pointer-events: auto; transition: opacity var(--dur) var(--ease); }
  .actions button { width: 30px; height: 28px; border: 0; border-radius: var(--radius-xs); background: transparent; color: var(--text-2); display: grid; place-items: center; }
  .actions button:hover { background: var(--bg-4); color: var(--text); }
  /* about two lines to start, grows with what's in it (images included) up to 200px, then scrolls */
  .body :global(.edit) { min-height: 58px; max-height: 200px; border-radius: var(--radius-sm); border: 1px solid var(--bg-4); background: var(--bg-1); padding: 8px; }
  .body :global(.edit:focus) { border-color: var(--focus-soft); }
  .err { margin: 2px 0 0; font-size: 12px; color: var(--danger-text); }
  .hint { font-size: 11px; color: var(--text-3); }
  .system { padding: 6px 18px 6px 30px; font-size: 13px; color: var(--text-3); }
  .call-line { display: flex; align-items: center; gap: 6px; }
  .call-line.missed { color: var(--danger-text); }
  .pin-line :global(svg) { vertical-align: -2px; }
  .pin-line .link-btn { color: var(--text-2); }
  .pin-line .link-btn:hover { color: var(--text); }
  /* the pin marker (#178): by the time, or in a grouped row's gutter, where the time would be */
  .pin-mark { display: inline-flex; color: var(--text-3); }
  .meta .pin-mark { align-self: center; }
  .gutter { display: flex; justify-content: center; padding-top: 4px; }
  .msg.mentioned { background: var(--accent-faint); box-shadow: inset 2px 0 0 var(--focus-ring); }
  .content :global(img.emoji) { width: 1.375em; height: 1.375em; vertical-align: -0.3em; object-fit: contain; }
  .content :global(img.emoji.big) { width: 48px; height: 48px; }
  .content :global(.emoji-gone) { color: var(--text-3); background: var(--bg-3); border-radius: 4px; padding: 0 4px; font-size: .9em; }
  .content :global(.emoji-gone.big) { font-size: 1.25em; }
  .content :global(.mention) { color: var(--accent-text); background: var(--accent-soft); border-radius: 4px; padding: 0 3px; font-weight: 600; font-size: calc(1em * var(--name-scale, 1)); line-height: calc(1.5em / var(--name-scale, 1)); vertical-align: baseline; cursor: pointer; clip-path: var(--name-clip); }
  /* a gradient name clips its background to the text: the pill background is on this wrapper instead */
  /* a thin gap between the @ and the name (#137) */
  .content :global(.mention .at) { margin-right: .15em; }
  .content :global(.mention-wrap) { background: var(--accent-soft); border-radius: 4px; padding: 0 3px; }
</style>
