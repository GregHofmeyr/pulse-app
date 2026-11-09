<script lang="ts">
  // A chat's pinned messages (#178): a popover under the header's pin button, as the quotes leaderboard is. Newest
  // pin first: who wrote it, a one-line preview, who pinned it and the time left, then Jump and Unpin. Fetched on
  // opening; pins, unpins and expiries from anyone keep it current (state.pins).
  import { clickOutside } from '../lib/actions'
  import { mentionLookup, showMentions } from '../lib/mentionStyles'
  import { livePin, pinTip } from '../lib/pins'
  import { serverNow } from '../lib/serverClock'
  import { displayName, nameOfIn } from '../lib/selectors'
  import { setPins } from '../lib/state'
  import { app, clock } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import Loading from './Loading.svelte'

  let { channelId, serverId, onJump, onClose }: {
    channelId: string
    serverId: string | null
    /** Show this message in the chat (the caller jumps to it). */
    onJump: (messageId: string) => void
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

  async function unpin(messageId: string) {
    error = ''
    try {
      await api.unpinMessage(messageId)
    } catch (e) {
      error = errorText(e)
    }
  }
</script>

<div class="pins-pop menu-surface pop-in" role="dialog" aria-label="Pinned messages" use:clickOutside={onClose}>
  <h2>Pinned messages</h2>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#if !pins}
    {#if !error}<Loading />{/if}
  {:else}
    {#each pins as p (p.message.id)}
      <div class="pin">
        <div class="text">
          <span class="author">{nameOf(p.message.author_id)}</span>
          <span class="said">{preview(p.message.content)}</span>
          <span class="note">{pinTip(nameOf(p.pin.pinned_by), p.pin.expires_at, now)}</span>
        </div>
        <button type="button" class="btn btn-quiet sm" onclick={() => onJump(p.message.id)}>Jump</button>
        <button type="button" class="btn btn-secondary sm" onclick={() => void unpin(p.message.id)}>Unpin</button>
      </div>
    {:else}
      <p class="none">Nothing pinned yet. Pin a message from its menu to keep it here.</p>
    {/each}
  {/if}
</div>

<style>
  /* the leaderboard's popover, kept inside a narrow column (a voice channel's chat panel) */
  .pins-pop { position: absolute; top: calc(100% + 6px); right: 12px; z-index: 30; width: min(400px, calc(100% - 24px)); max-height: 70vh; overflow-y: auto; padding: 14px; gap: 2px; color: var(--text); }
  h2 { margin: 0 0 8px; font-size: 15px; }
  .pin { display: flex; align-items: center; gap: 6px; padding: 6px 8px; border-radius: var(--radius-sm); }
  .pin:hover { background: var(--bg-4); }
  .text { flex: 1; min-width: 0; display: flex; flex-direction: column; font-size: 13px; }
  .author { font-weight: 600; color: var(--text); }
  .said { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text-2); }
  .note { color: var(--text-3); font-size: 12px; }
  .sm { height: 26px; padding: 0 8px; font-size: 12px; }
  .none { margin: 2px 8px; color: var(--text-3); font-size: 13px; }
  .error { margin: 2px 8px; color: var(--danger-text); font-size: 13px; }
</style>
