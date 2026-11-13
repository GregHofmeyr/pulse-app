<script lang="ts">
  // A chat's pinned messages (#178): a popover under the header's pin button, as the quotes leaderboard is. Newest
  // pin first: who wrote it, a one-line preview, who pinned it and the time left, then Jump and Unpin. In a quotes
  // channel a quote reads as who said it and its words; in a clips channel a clip as its thumbnail and caption, with
  // Open (its panel) for Jump (#257). Fetched on opening; pins, unpins and expiries from anyone keep it current
  // (state.pins).
  import { clickOutside } from '../lib/actions'
  import { mentionLookup, showMentions } from '../lib/mentionStyles'
  import { livePin, pinsTitle, pinTip } from '../lib/pins'
  import { serverNow } from '../lib/serverClock'
  import { displayName, nameOfIn } from '../lib/selectors'
  import { setPins } from '../lib/state'
  import { app, clock } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import Loading from './Loading.svelte'
  import { shownText } from '../lib/poll'
  import { formatDuration, gameName } from '../lib/clips'
  import { mediaUrl } from '../lib/emoji'
  import { speakerOf } from '../lib/quotes'
  import type { Message } from '../lib/protocol/Message'

  let { channelId, serverId, onJump, onClose }: {
    channelId: string
    serverId: string | null
    /** Show this message: the caller jumps to it in the chat, or (a clip, which has no history to jump in) opens
     *  its panel (#257). */
    onJump: (message: Message) => void
    onClose: () => void
  } = $props()

  let error = $state('')
  // Fetched when first opened, then kept live by the pin events; fetched again whenever the list is gone (a
  // reconnect's Ready forgets it).
  const held = $derived(app.state.pins[channelId])
  let loading: string | null = null
  $effect(() => {
    const id = channelId
    if (held || loading === id) return
    loading = id
    error = ''
    api.listPins(id).then(
      (list) => {
        loading = null
        if (id === channelId) app.state = setPins(app.state, id, list)
      },
      (e) => {
        loading = null
        if (id === channelId) error = errorText(e)
      },
    )
  })

  const nameOf = $derived(nameOfIn(() => app.state, serverId))
  const lookup = $derived(mentionLookup(app.state))
  // the clock moves every 30 s: time left follows it, and a pin that runs out leaves before the sweep's event (by
  // the server's clock, #258)
  const now = $derived((void clock.nowIso, serverNow()))
  const pins = $derived(held ? held.filter((p) => livePin({ ...p.message, pin: p.pin }, now)) : null)
  const preview = (content: string) => showMentions(content, lookup, (id) => displayName(app.state, serverId, id))
  // A quote reads as who said it and its words; a clip as its caption (else its game, else "Clip · 0:30") by its
  // poster, beside its thumbnail (#257); a message as its author and a one-line preview.
  const kind = $derived(app.state.channels[channelId]?.kind)
  function who(m: Message): string {
    if (!m.quote) return nameOf(m.author_id)
    return [...new Set(m.quote.lines.map((l) => speakerOf(app.state, serverId ?? '', l).name))].join(', ')
  }
  function what(m: Message): string {
    if (m.quote) return m.quote.lines.map((l) => (l.media ? (l.caption ? `[image] ${l.caption}` : '[image]') : `“${l.text ?? ''}”`)).join(' ')
    if (m.clip) return m.content.trim() || gameName(app.state, serverId ?? '', m) || `Clip · ${formatDuration(m.clip.duration_ms)}`
    return preview(shownText(m))
  }
  const none = $derived(
    kind === 'quotes' ? 'Pin a quote from its right-click menu to keep it here.'
    : kind === 'clips' ? 'Pin a clip from its ⋯ menu to keep it here.'
    : 'Pin a message from its menu to keep it here.',
  )

  async function unpin(messageId: string) {
    error = ''
    try {
      await api.unpinMessage(messageId)
    } catch (e) {
      error = errorText(e)
    }
  }
</script>

<div class="pins-pop menu-surface pop-in" role="dialog" aria-label={pinsTitle(kind)} use:clickOutside={onClose}>
  <h2>{pinsTitle(kind)}</h2>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#if !pins}
    {#if !error}<Loading />{/if}
  {:else}
    {#each pins as p (p.message.id)}
      {@const m = p.message}
      <div class="pin">
        {#if m.clip}<img class="thumb" src={mediaUrl(m.clip.thumb_hash)} alt="" loading="lazy" decoding="async" />{/if}
        <div class="text">
          <span class="author">{who(m)}</span>
          <span class="said">{what(m)}</span>
          <span class="note">{pinTip(nameOf(p.pin.pinned_by), p.pin.expires_at, now)}</span>
        </div>
        <button type="button" class="btn btn-quiet sm" onclick={() => onJump(m)}>{m.clip ? 'Open' : 'Jump'}</button>
        <button type="button" class="btn btn-secondary sm" onclick={() => void unpin(m.id)}>Unpin</button>
      </div>
    {:else}
      <p class="none">Nothing pinned yet. {none}</p>
    {/each}
  {/if}
</div>

<style>
  /* the leaderboard's popover, kept inside a narrow column (a voice channel's chat panel) */
  .pins-pop { position: absolute; top: calc(100% + 6px); right: 12px; z-index: 30; width: min(400px, calc(100% - 24px)); max-height: 70vh; overflow-y: auto; padding: 14px; gap: 2px; color: var(--text); }
  h2 { margin: 0 0 8px; font-size: 15px; }
  .pin { display: flex; align-items: center; gap: 6px; padding: 6px 8px; border-radius: var(--radius-sm); }
  .pin:hover { background: var(--bg-4); }
  /* a clip's thumbnail (#257), 16:9 at the row's three lines */
  .thumb { flex-shrink: 0; width: 80px; height: 45px; object-fit: cover; border-radius: var(--radius-xs); background: var(--bg-3); }
  .text { flex: 1; min-width: 0; display: flex; flex-direction: column; font-size: 13px; }
  .author { font-weight: 600; color: var(--text); }
  .said { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text-2); }
  .note { color: var(--text-3); font-size: 12px; }
  .sm { height: 26px; padding: 0 8px; font-size: 12px; }
  .none { margin: 2px 8px; color: var(--text-3); font-size: 13px; }
  .error { margin: 2px 8px; color: var(--danger-text); font-size: 13px; }
</style>
