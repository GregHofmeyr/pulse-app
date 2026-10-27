<script lang="ts">
  import { keyboardFocused } from '../lib/focus'
  import Avatar from './Avatar.svelte'
  import EmojiPopover from './EmojiPopover.svelte'
  import Icon from './Icon.svelte'
  import LineMenu from './LineMenu.svelte'
  import ReactionRow from './ReactionRow.svelte'
  import VoterStack from './VoterStack.svelte'
  import { modal } from '../lib/actions'
  import { animatedEmoji, stillSrc } from '../lib/animate.svelte'
  import { mediaUrl, reactionKey, type Std } from '../lib/emoji'
  import { menuPoint } from '../lib/participantMenu'
  import { hasNewReplies, nameKey, speakerOf } from '../lib/quotes'
  import { blockedReason, pickAdds, react } from '../lib/reactions'
  import { app } from '../lib/store.svelte'
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
    onOpenReplies = () => {},
  }: {
    message: Message
    serverId: string
    meId: string
    nameOf: (userId: string | null) => string
    /** Absent where the replies panel pins the quote itself: no Reply entry then. */
    onReply?: () => void
    onEdit: () => void
    onDelete: () => Promise<void>
    onOpenReplies?: () => void
  } = $props()

  const quote = $derived(message.quote)
  const mine = $derived(message.author_id === meId)
  const when = $derived(new Date(message.created_at).toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' }))
  // A quote carries one reaction per person: suggest the first three of your usual picks it doesn't have yet.
  const suggest = $derived(app.state.frequentEmoji.filter((k) => !message.reactions.some((r) => r.emoji === k) && !blockedReason(app.state, k)).slice(0, 3))
  const members = $derived(
    (app.state.members[serverId] ?? []).map((m) => ({ id: m.user.id, name: m.nickname ?? m.user.username })),
  )

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

  // --- the line menu ---
  let menu = $state<{ line: QuoteLine; x: number; y: number; opener: HTMLElement } | null>(null)
  function openMenu(line: QuoteLine, e: MouseEvent | KeyboardEvent, anchor: HTMLElement) {
    // clicking the opener again closes its menu
    if (e.type === 'click' && menu?.opener === anchor) menu = null
    else menu = { line, ...menuPoint(e, anchor), opener: anchor }
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
  const copyable = (line: QuoteLine) => !!(line.text ?? line.caption)
  const copy = (line: QuoteLine) => attempt(() => navigator.clipboard.writeText(line.text ?? line.caption ?? ''))
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

  // --- the full-size image ---
  let open = $state<{ media: LineMedia; caption: string | null } | null>(null)
</script>

{#if quote}
  <!-- focus anywhere inside shows the action bar (hover alone left keyboard users without it) -->
  <div class="quote" data-message-id={message.id} onfocusin={(e) => { if (keyboardFocused(e.target)) focusIn = true }} onfocusout={(e) => { if (!e.currentTarget.contains(e.relatedTarget as Node | null)) focusIn = false }}>
    <div class="row">
      <Avatar id={message.author_id ?? ''} name={nameOf(message.author_id)} size={40} />
      <div class="body">
        <div class="meta"><strong>{nameOf(message.author_id)}</strong><span class="time">{when}</span></div>
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
                    <button type="button" class="picbtn" aria-label="Open image full size" onclick={() => (open = { media: line.media!, caption: line.caption })}>
                      <img class="pic" src={stillSrc(line.media)} alt={line.caption ?? 'Quoted image'} draggable="false"
                        width={line.media.width} height={line.media.height} {@attach animatedEmoji(line.media)} />
                    </button>
                    {#if line.caption}<p class="caption">{line.caption}</p>{/if}
                  {:else}
                    <span class="text">“{line.text}”</span>
                  {/if}
                </div>
                <div class="by">
                  <span class="speaker">{who.name}</span>
                  {#if line.awards > 0}
                    <span class="trophy" aria-hidden="true">🏆 {line.awards}</span>
                    <VoterStack userIds={line.awarders} {nameOf} />
                  {/if}
                  <button type="button" class="lbtn" aria-label="Line actions" aria-haspopup="menu" aria-expanded={menu?.line.id === line.id} data-menu-toggle
                    onclick={(e) => openMenu(line, e, e.currentTarget)}><Icon name="dots" size={15} /></button>
                </div>
              </div>
            {/each}
          </div>
          {#each quote.wins as year (year)}
            <span class="win">Quote of the Year {year}</span>
          {/each}
          {#if message.edited_at}<span class="edited">(edited)</span>{/if}
        {/if}
        <ReactionRow {message} {meId} {nameOf} onError={showError} onePerPerson {suggest} />
        {#if quote.reply_count > 0}
          <button type="button" class="replies" onclick={onOpenReplies}>
            View {quote.reply_count} {quote.reply_count === 1 ? 'reply' : 'replies'}
            {#if hasNewReplies(message)}<span class="new">new</span>{/if}
          </button>
        {/if}
        {#if inlineError.text}<p class="err" role="alert">{inlineError.text}</p>{/if}
      </div>
      {#if !message.deleted}
        <div class="actions" class:open={picking || focusIn}>
          <button bind:this={smileBtn} aria-label="Add reaction" data-menu-toggle onclick={() => (picking = !picking)}><Icon name="smile" size={15} /></button>
          {#if onReply}<button aria-label="Reply" onclick={onReply}><Icon name="reply" size={15} /></button>{/if}
          {#if mine}
            <button aria-label="Edit" onclick={onEdit}><Icon name="pencil" size={15} /></button>
            <button aria-label="Delete" onclick={() => void attempt(onDelete)}><Icon name="trash" size={15} /></button>
          {/if}
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
        onAward={() => toggleAward(line)} onCopy={copyable(line) ? () => copy(line) : null} onLink={(id) => link(line, id)} onUnlink={() => unlink(line)}
        onClose={() => (menu = null)} />
    {/key}
  {/if}
  {#if open}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div class="lightbox" role="dialog" aria-modal="true" aria-label={open.caption ?? 'Quoted image'} tabindex="-1" use:modal={() => (open = null)} onclick={() => (open = null)}>
      <img src={stillSrc(open.media)} alt={open.caption ?? 'Quoted image'} draggable="false" {@attach animatedEmoji(open.media)} />
      <button type="button" class="close" aria-label="Close" onclick={() => (open = null)}><Icon name="x" size={18} /></button>
    </div>
  {/if}
{/if}

<style>
  .quote { padding: 8px 18px 2px; position: relative; }
  .quote:hover { background: rgba(255, 255, 255, .02); }
  .row { display: flex; gap: 14px; }
  .body { flex: 1; min-width: 0; }
  .meta { display: flex; align-items: baseline; gap: 8px; margin-bottom: 4px; }
  .meta strong { font-size: 14.5px; }
  .time { font-size: 11px; color: var(--text-3); }
  .deleted { margin: 0; font-style: italic; color: var(--text-3); }
  .lines { display: flex; flex-direction: column; gap: 2px; border-left: 3px solid var(--quote-green-deep); padding-left: 10px; }
  .line { display: flex; align-items: baseline; justify-content: space-between; gap: 16px; padding: 2px 6px; margin-left: -6px; border-radius: 8px; outline: none; }
  .line:hover, .line:focus-visible, .line:focus-within { background: rgba(255, 255, 255, .03); }
  .line:focus-visible { box-shadow: 0 0 0 2px var(--accent); }
  .said { min-width: 0; color: #dcdee5; line-height: 1.5; overflow-wrap: anywhere; }
  .by { display: flex; align-items: center; gap: 8px; flex-shrink: 0; color: var(--text-2); }
  .speaker::before { content: '— '; color: var(--text-3); }
  .trophy { color: var(--quote-green); font-size: 12.5px; font-weight: 600; }
  .lbtn { width: 24px; height: 24px; border: 0; border-radius: 6px; background: transparent; color: var(--text-3); display: grid; place-items: center; opacity: 0; }
  .line:hover .lbtn, .line:focus-within .lbtn { opacity: 1; }
  .lbtn:hover { background: var(--bg-3); color: var(--text); }
  .picbtn { display: block; padding: 0; border: 0; background: none; cursor: zoom-in; border-radius: 8px; }
  .pic { display: block; max-width: 360px; max-height: 360px; width: auto; height: auto; border-radius: 8px; object-fit: contain; }
  .caption { margin: 4px 0 0; font-size: 13px; color: var(--text-2); }
  .win { display: inline-block; margin: 6px 6px 0 0; padding: 1px 8px; border-radius: 10px; background: var(--quote-green); color: var(--on-quote-green); font-size: 11.5px; font-weight: 700; }
  .edited { font-size: 11px; color: var(--text-3); }
  .replies { margin-top: 4px; padding: 0; border: 0; background: none; color: var(--accent); font-size: 13px; display: inline-flex; align-items: center; gap: 6px; }
  .replies:hover { text-decoration: underline; }
  .new { padding: 0 6px; border-radius: 8px; background: var(--quote-green); color: var(--on-quote-green); font-size: 10px; font-weight: 700; text-transform: uppercase; }
  .err { margin: 2px 0 0; font-size: 12px; color: var(--danger); }
  .actions { position: absolute; top: -10px; right: 18px; display: flex; opacity: 0; pointer-events: none; background: var(--bg-3); border: 1px solid var(--bg-4); border-radius: 8px; }
  .quote:hover .actions, .actions.open { opacity: 1; pointer-events: auto; }
  .actions button { width: 30px; height: 28px; border: 0; background: transparent; color: var(--text-2); display: grid; place-items: center; }
  .actions button:hover { color: var(--text); }
  .lightbox { position: fixed; inset: 0; z-index: 70; background: rgba(10, 11, 13, .85); display: grid; place-items: center; cursor: zoom-out; outline: none; }
  .lightbox img { max-width: 92vw; max-height: 92vh; object-fit: contain; border-radius: 8px; }
  .close { position: absolute; top: 16px; right: 16px; width: 36px; height: 36px; border: 0; border-radius: 10px; background: var(--bg-3); color: var(--text); display: grid; place-items: center; }
</style>
