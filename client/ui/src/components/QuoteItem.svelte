<script lang="ts">
  import { actionBarFocus } from '../lib/focus'
  import { saveImage } from '../lib/clipDownload'
  import Avatar from './Avatar.svelte'
  import ClipDownload from './ClipDownload.svelte'
  import DeleteButton from './DeleteButton.svelte'
  import EmojiPopover from './EmojiPopover.svelte'
  import Icon from './Icon.svelte'
  import LineMenu from './LineMenu.svelte'
  import Name from './Name.svelte'
  import QuoteImage from './QuoteImage.svelte'
  import ReactionRow from './ReactionRow.svelte'
  import VoterStack from './VoterStack.svelte'
  import { modal } from '../lib/actions'
  import { mediaUrl, reactionKey, type Std } from '../lib/emoji'
  import { isMenuKey, menuPoint } from '../lib/participantMenu'
  import { messagePerson } from '../lib/messagePerson'
  import MemberMenu from './MemberMenu.svelte'
  import { hasNewReplies, nameKey, quoteText, speakerOf } from '../lib/quotes'
  import { blockedReason, pickAdds, react } from '../lib/reactions'
  import { app, clock } from '../lib/store.svelte'
  import { serverPeople } from '../lib/selectors'
  import { livePin, pinTip } from '../lib/pins'
  import { refreshPins } from '../lib/pinSync.svelte'
  import { serverNow } from '../lib/serverClock'
  import type { PinDuration } from '../lib/protocol/PinDuration'
  import { api, errorText } from '../lib/tauri'
  import type { CustomEmoji } from '../lib/protocol/CustomEmoji'
  import type { LineMedia } from '../lib/protocol/LineMedia'
  import type { Message } from '../lib/protocol/Message'
  import type { QuoteLine } from '../lib/protocol/QuoteLine'
  import { InlineError } from '../lib/inlineError.svelte'

  let {
    message,
    serverId,
    meId,
    nameOf,
    onReply,
    onEdit,
    onDelete,
    onOpenReplies,
    unsentReplies = 0,
    compact = false,
  }: {
    message: Message
    serverId: string
    meId: string
    nameOf: (userId: string | null) => string
    /** Absent where the replies panel pins the quote itself: no Reply entry then. */
    onReply?: () => void
    /** Absent (with onDelete) where the quote can't be changed: no Edit or Delete then. */
    onEdit?: () => void
    onDelete?: () => Promise<void>
    /** Absent where the replies panel pins the quote: no "View N replies" there. */
    onOpenReplies?: () => void
    /** Your replies to it not sent yet (pending or failed), shown while its panel is closed; opens it. */
    unsentReplies?: number
    /** Just the lines (Quote of the Year's options): no poster, reactions, replies or quote actions.
     *  Each line keeps its award button and right-click menu (link, copy): those show wherever the line does. */
    compact?: boolean
  } = $props()

  const quote = $derived(message.quote)
  const mine = $derived(message.author_id === meId)
  const when = $derived(new Date(message.created_at).toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' }))
  // A quote carries one reaction per person: suggest the first three of your usual picks it doesn't have yet.
  const suggest = $derived(app.state.frequentEmoji.filter((k) => !message.reactions.some((r) => r.emoji === k) && !blockedReason(app.state, k)).slice(0, 3))
  const members = $derived(serverPeople(app.state, serverId))

  const inlineError = new InlineError()
  const showError = inlineError.show
  $effect(() => inlineError.stop)
  async function attempt(f: () => Promise<unknown>) {
    try {
      await f()
    } catch (e) {
      showError(errorText(e))
    }
  }

  // --- the line's right-click menu (#244: no ⋯ button; awarding is the line's own button, copy is the whole quote) ---
  let menu = $state<{ line: QuoteLine; x: number; y: number; opener: HTMLElement } | null>(null)
  function openMenu(line: QuoteLine, e: MouseEvent | KeyboardEvent, anchor: HTMLElement) {
    // a line with nothing to offer (my own picture, say) opens no empty menu
    if (!hasActions(line)) return
    menu = { line, ...menuPoint(e, anchor), opener: anchor }
  }
  function lineKey(e: KeyboardEvent, line: QuoteLine) {
    if (e.target !== e.currentTarget) return
    if (e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey)) {
      e.preventDefault()
      openMenu(line, e, e.currentTarget as HTMLElement)
    }
  }
  const awardNote = (n: number) => (n > 0 ? `, ${n} ${n === 1 ? 'award' : 'awards'}` : '')
  const speaker = (line: QuoteLine) => speakerOf(app.state, serverId, line)
  const awardOf = (line: QuoteLine): 'give' | 'take' | null =>
    speaker(line).userId === meId ? null : line.awarded ? 'take' : 'give'
  const linkOf = (line: QuoteLine): 'link' | 'unlink' | null =>
    line.speaker_id ? null : speaker(line).userId ? 'unlink' : 'link'
  const toggleAward = (line: QuoteLine) => attempt(() => (line.awarded ? api.takeAward(line.id) : api.giveAward(line.id)))
  const awardTip = (line: QuoteLine) => (line.awarded ? 'Take back award' : 'Give quote award')
  /** The whole quote as text (every line, with who said it); null when it has no words to copy. */
  const copyText = $derived(quote ? quoteText(app.state, serverId, quote.lines) : null)
  // --- pins (#257): the whole quote pins from any line's menu, as Copy quote does; not a QotY option (compact) ---
  // the clock moves every 30 s: a timed pin's time left (and its end) follow it, by the server's clock (#258)
  const now = $derived((void clock.nowIso, serverNow()))
  const pin = $derived(livePin(message, now))
  const pinMark = $derived(pin ? pinTip(nameOf(pin.pinned_by), pin.expires_at, now) : '')
  const pinItem = $derived<'pin' | 'unpin' | null>(compact || message.deleted ? null : pin ? 'unpin' : 'pin')
  // A refused pin or unpin means the pin here was out of date: show why, and fetch the chat's pins to set it right.
  const report = (p: Promise<unknown>) =>
    p.catch((e) => {
      showError(errorText(e))
      void refreshPins(message.channel_id)
    })
  const pinQuote = (d: PinDuration) => report(api.pinMessage(message.id, d))
  const unpinQuote = () => report(api.unpinMessage(message.id))
  /** Whether the line's menu would have anything in it: award, copy, save (an image), pin, or link. */
  const hasActions = (line: QuoteLine) => !!awardOf(line) || !!copyText || !!line.media || !!pinItem || !!linkOf(line)
  const save = (media: LineMedia, caption: string | null) => void saveImage(media, caption, showError)
  // the Copy quote button's tooltip says "Copied" for a moment after
  let copied = $state(false)
  let copiedTimer: ReturnType<typeof setTimeout> | undefined
  $effect(() => () => clearTimeout(copiedTimer))
  const copy = () =>
    attempt(async () => {
      if (!copyText) return
      await navigator.clipboard.writeText(copyText)
      copied = true
      clearTimeout(copiedTimer)
      copiedTimer = setTimeout(() => (copied = false), 2000)
    })
  const link = (line: QuoteLine, userId: string) => attempt(() => api.linkSpeaker(serverId, { name: line.speaker_name ?? '', user_id: userId }))
  const unlink = (line: QuoteLine) => attempt(() => api.unlinkSpeaker(serverId, nameKey(line.speaker_name ?? '')))

  // --- reacting (one reaction per person) ---
  let focusIn = $state(false)
  let picking = $state(false)
  let smileBtn: HTMLButtonElement | undefined = $state()
  function pick(e: Std | CustomEmoji, { keep }: { keep: boolean }) {
    if (!keep) picking = false
    const key = reactionKey(e)
    void react(() => app.state, (s) => (app.state = s), message.channel_id, message.id, key, pickAdds(message.reactions, key, keep), meId, showError, true)
  }

  // --- names: the poster's avatar animates while the pointer is over the quote ---
  let over = $state(false)
  const sid = $derived(serverId || null)

  // --- a person's menu (#135): right-click (or ContextMenu / Shift+F10 on) the poster's name or avatar, or a member
  // speaker's name (which then isn't the line's menu) ---
  let personMenu = $state<{ userId: string; x: number; y: number; opener: HTMLElement } | null>(null)
  function openPersonMenu(userId: string | null, e: MouseEvent | KeyboardEvent, zone: HTMLElement) {
    if (!userId) return
    if (e instanceof KeyboardEvent && !isMenuKey(e)) return
    e.preventDefault()
    e.stopPropagation()
    const opener = zone.querySelector<HTMLElement>('button') ?? zone
    personMenu = { userId, ...menuPoint(e, opener), opener }
  }

  // --- the full-size image ---
  let open = $state<{ media: LineMedia; caption: string | null } | null>(null)
</script>

{#if quote}
  <!-- focus anywhere inside shows the action bar (hover alone left keyboard users without it) -->
  <div class="quote" class:compact data-message-id={message.id} {...actionBarFocus((v) => (focusIn = v))}
    onpointerenter={() => (over = true)} onpointerleave={() => (over = false)}>
    <div class="row">
      <!-- the person menu is a pointer shortcut here; the keyboard route is ContextMenu / Shift+F10 on the avatar or name -->
      {#if !compact}<span class="author" role="presentation" oncontextmenu={(e) => openPersonMenu(message.author_id, e, e.currentTarget)} onkeydown={(e) => openPersonMenu(message.author_id, e, e.currentTarget)}><Avatar id={message.author_id ?? ''} name={nameOf(message.author_id)} size={40} interactive={!!message.author_id} serverId={sid} hovered={() => over} /></span>{/if}
      <div class="body">
        {#if !compact}<div class="meta"><strong role="presentation" oncontextmenu={(e) => openPersonMenu(message.author_id, e, e.currentTarget)} onkeydown={(e) => openPersonMenu(message.author_id, e, e.currentTarget)}>{#if message.author_id}<Name userId={message.author_id} serverId={sid} text={nameOf(message.author_id)} />{:else}{nameOf(null)}{/if}</strong><span class="time">{when}</span>{#if pin}<span class="pin-mark" data-tip={pinMark}><Icon name="pin" size={14} /></span>{/if}</div>{/if}
        {#if message.deleted}
          <p class="deleted">quote deleted</p>
        {:else}
          <div class="lines">
            {#each quote.lines as line (line.id)}
              {@const who = speaker(line)}
              <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
              <div class="line" role="group" tabindex="0" aria-label="{who.name}: {line.text ?? line.caption ?? 'image'}{awardNote(line.awards)}"
                oncontextmenu={(e) => { e.preventDefault(); openMenu(line, e, e.currentTarget) }}
                onkeydown={(e) => lineKey(e, line)}>
                <div class="said">
                  {#if line.media}
                    <div class="pic">
                      <button type="button" class="picbtn" aria-label="Open image full size" onclick={() => (open = { media: line.media!, caption: line.caption })}>
                        <QuoteImage media={line.media} caption={line.caption} />
                      </button>
                      <ClipDownload label="Save image" tip="Save image" onclick={() => save(line.media!, line.caption)} />
                    </div>
                    {#if line.caption}<p class="caption">{line.caption}</p>{/if}
                  {:else}
                    <span class="text">“{line.text}”</span>
                  {/if}
                </div>
                <div class="by">
                  <!-- a member (or a name linked to one) is styled and opens their card; a plain name stays text -->
                  <span class="speaker" role="presentation" oncontextmenu={(e) => openPersonMenu(who.userId, e, e.currentTarget)} onkeydown={(e) => openPersonMenu(who.userId, e, e.currentTarget)}>{#if who.userId}<Name userId={who.userId} serverId={sid} text={who.name} />{:else}<span class="name-clip">{who.name}</span>{/if}</span>
                  {#if line.awards > 0}
                    <span class="trophy" aria-hidden="true">🏆 {line.awards}</span>
                    <VoterStack userIds={line.awarders} {nameOf} />
                  {/if}
                  <!-- the award, one click to give or take back (#244); none on your own line -->
                  {#if awardOf(line)}
                    <button type="button" class="icon-btn icon-btn-xs award" aria-label={awardTip(line)} data-tip={awardTip(line)} aria-pressed={line.awarded}
                      onclick={() => toggleAward(line)}><Icon name="trophy" size={16} /></button>
                  {/if}
                </div>
              </div>
            {/each}
          </div>
          {#each quote.wins as year (year)}
            <span class="win">Quote of the Year {year}</span>
          {/each}
          {#if message.edited_at}<span class="edited">(edited)</span>{/if}
        {/if}
        {#if !compact}<ReactionRow {message} {meId} {nameOf} onError={showError} onePerPerson {suggest} />{/if}
        {#if quote.reply_count > 0 && onOpenReplies && !compact}
          <button type="button" class="replies" onclick={onOpenReplies}>
            View {quote.reply_count} {quote.reply_count === 1 ? 'reply' : 'replies'}
            {#if hasNewReplies(message)}<span class="new">new</span>{/if}
          </button>
        {/if}
        {#if unsentReplies > 0 && onOpenReplies && !compact}
          <button type="button" class="unsent-reply" onclick={onOpenReplies}>
            <Icon name="reply" size={14} />{unsentReplies === 1 ? 'Unsent reply' : `${unsentReplies} unsent replies`}
          </button>
        {/if}
        {#if inlineError.text}<p class="err" role="alert">{inlineError.text}</p>{/if}
      </div>
      {#if !message.deleted && !compact}
        <div class="actions" class:open={picking || focusIn}>
          <button bind:this={smileBtn} aria-label="Add reaction" data-menu-toggle onclick={() => (picking = !picking)}><Icon name="smile" size={16} /></button>
          {#if onReply}<button aria-label="Reply" onclick={onReply}><Icon name="reply" size={16} /></button>{/if}
          {#if copyText}<button aria-label="Copy quote" data-tip={copied ? 'Copied' : 'Copy quote'} onclick={() => void copy()}><Icon name="copy" size={16} /></button>{/if}
          {#if mine && onEdit}<button aria-label="Edit" onclick={onEdit}><Icon name="pencil" size={16} /></button>{/if}
          {#if mine && onDelete}<DeleteButton what="quote" note="Its replies go too." onConfirm={() => void attempt(onDelete)} />{/if}
        </div>
        {#if picking && smileBtn}
          <EmojiPopover anchor={smileBtn} onPick={pick} onClose={() => (picking = false)} />
        {/if}
      {/if}
    </div>
  </div>
  {#if menu}
    {@const m = menu}
    {@const line = m.line}
    {#key m}
      <LineMenu x={m.x} y={m.y} opener={m.opener} award={awardOf(line)} link={linkOf(line)} {members}
        onAward={() => toggleAward(line)} onCopy={copyText ? copy : null}
        onSave={line.media ? () => save(line.media!, line.caption) : null} pin={pinItem} onPin={pinQuote} onUnpin={unpinQuote}
        onLink={(id) => link(line, id)} onUnlink={() => unlink(line)}
        onClose={() => (menu = null)} />
    {/key}
  {/if}
  {#if personMenu}
    {#key personMenu}<MemberMenu serverId={sid} userId={personMenu.userId} x={personMenu.x} y={personMenu.y} opener={personMenu.opener} onMessage={messagePerson} onClose={() => (personMenu = null)} />{/key}
  {/if}
  {#if open}
    {@const shown = open}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div class="lightbox" role="dialog" aria-modal="true" aria-label={open.caption ?? 'Quoted image'} tabindex="-1" use:modal={() => (open = null)} onclick={() => (open = null)}>
      <div class="pic">
        <QuoteImage media={open.media} caption={open.caption} full />
        <!-- saving leaves the view open -->
        <ClipDownload label="Save image" tip="Save image" onclick={(e) => { e.stopPropagation(); save(shown.media, shown.caption) }} />
      </div>
      <button type="button" class="close icon-btn round modal-close" aria-label="Close" onclick={() => (open = null)}><Icon name="x" size={16} /></button>
    </div>
  {/if}
{/if}

<style>
  /* --bar-room: the quick-action bar's room at narrow widths, as in MessageItem (#237) */
  .author { display: contents; }
  .quote { --bar-room: clamp(0px, calc((1200px - 100vw) / 25), 4px); padding: 8px 18px 2px; position: relative; }
  .quote:hover { background: var(--hover-tint); }
  .quote.compact { padding: 0; }
  .quote.compact:hover { background: none; }
  .row { display: flex; gap: 14px; }
  .body { flex: 1; min-width: 0; }
  .meta { display: flex; align-items: baseline; gap: 8px; margin-bottom: 4px; }
  .meta strong { font-size: 14.5px; }
  .time { font-size: 11px; color: var(--text-3); }
  /* the pin marker (#257), as a message's (MessageItem .pin-mark): by the time */
  .pin-mark { display: inline-flex; align-self: center; color: var(--text-3); }
  .deleted { margin: 0; font-style: italic; color: var(--text-3); }
  .lines { display: flex; flex-direction: column; gap: 2px; border-left: 3px solid var(--quote-green-deep); padding-left: 10px; }
  .line { display: flex; align-items: baseline; justify-content: space-between; gap: 16px; padding: 2px 6px; margin-left: -6px; border-radius: var(--radius-sm); outline: none; }
  .line:hover, .line:focus-visible, .line:focus-within { background: var(--hover-tint-strong); }
  .line:focus-visible { box-shadow: 0 0 0 2px var(--focus-ring); }
  .said { min-width: 0; color: var(--text-body); line-height: 1.5; overflow-wrap: anywhere; }
  .by { display: flex; align-items: center; gap: 8px; flex-shrink: 0; color: var(--text-2); }
  .speaker::before { content: '— '; color: var(--text-3); }
  .trophy { color: var(--quote-green-text); font-size: 12px; font-weight: 600; }
  /* the award button (#244): shows on every line while the pointer is over the quote, and stays lit once you've given
     it. To try it always-on, drop the `opacity: 0`; to show it only on the line under the pointer, use .line:hover. */
  .award { opacity: 0; }
  .quote:hover .award, .line:focus-within .award, .award[aria-pressed='true'] { opacity: 1; }
  .award[aria-pressed='true'] { color: var(--quote-green-text); }
  /* the image and the clips' download icon over its corner (#206), shown while the pointer is over it */
  .pic { position: relative; width: fit-content; max-width: 100%; }
  .pic:hover :global(.clip-dl) { opacity: 1; }
  .picbtn { display: block; padding: 0; border: 0; background: none; cursor: zoom-in; border-radius: var(--radius-sm); }
  .caption { margin: 4px 0 0; font-size: 13px; color: var(--text-2); }
  .win { display: inline-block; margin: 6px 6px 0 0; padding: 1px 8px; border-radius: var(--radius-md); background: var(--quote-green); color: var(--on-quote-green); font-size: 11px; font-weight: 700; }
  .edited { font-size: 11px; color: var(--text-3); }
  .replies { margin-top: 4px; padding: 0; border: 0; background: none; color: var(--accent-text); font-size: 13px; display: inline-flex; align-items: center; gap: 6px; }
  .replies:hover { text-decoration: underline; }
  .new { padding: 0 6px; border-radius: var(--radius-sm); background: var(--quote-green); color: var(--on-quote-green); font-size: 10px; font-weight: 700; text-transform: uppercase; }
  .unsent-reply { margin: 4px 0 0 10px; padding: 0; border: 0; background: none; color: var(--danger-text); font-size: 13px; display: inline-flex; align-items: center; gap: 5px; }
  .unsent-reply:hover { text-decoration: underline; }
  .err { margin: 2px 0 0; font-size: 12px; color: var(--danger-text); }
  .actions { position: absolute; top: calc(-10px + 1.5 * var(--bar-room)); right: 18px; display: flex; opacity: 0; pointer-events: none; background: var(--bg-3); border: 1px solid var(--bg-4); border-radius: var(--radius-sm); will-change: opacity; }
  /* fades in, hides at once (#173): sweeping down a channel never leaves half-faded bars behind */
  .quote:hover .actions, .actions.open { opacity: 1; pointer-events: auto; transition: opacity var(--dur) var(--ease); }
  .actions button { width: 30px; height: 28px; border: 0; border-radius: var(--radius-xs); background: transparent; color: var(--text-2); display: grid; place-items: center; }
  .actions button:hover { background: var(--bg-4); color: var(--text); }
  .lightbox { position: fixed; inset: 0; z-index: 70; background: var(--scrim-strong); display: grid; place-items: center; cursor: zoom-out; outline: none; }
  /* the dialogs' round close X, in the corner of the lightbox */
  .close { position: absolute; top: 16px; right: 16px; }
</style>
