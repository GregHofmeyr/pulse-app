<script lang="ts">
  import { untrack } from 'svelte'
  import type { Message } from '../lib/protocol/Message'
  import type { CustomEmoji } from '../lib/protocol/CustomEmoji'
  import type { Std } from '../lib/emoji'
  import { reactionKey, twemojiUrl } from '../lib/emoji'
  import { animatedEmoji, stillSrc } from '../lib/animate.svelte'
  import { react, tooltipText } from '../lib/reactions'
  import { app } from '../lib/store.svelte'
  import { api } from '../lib/tauri'
  import EmojiPopover from './EmojiPopover.svelte'
  import Icon from './Icon.svelte'

  let { message, meId, nameOf, onError = () => {} }: {
    message: Message
    meId: string
    nameOf: (userId: string | null) => string
    /** Where a failed add/remove is reported (the channel's error line). */
    onError?: (msg: string) => void
  } = $props()

  let tipFor = $state<string | null>(null)
  let tipText = $state('')
  let picking = $state(false)
  let addBtn: HTMLButtonElement | undefined = $state()

  const customOf = (key: string): CustomEmoji | undefined => app.state.customEmoji[key.slice(7)]

  function toggle(key: string, add: boolean) {
    void react(() => app.state, (s) => (app.state = s), message.channel_id, message.id, key, add, meId, onError)
  }

  let tipReq = 0
  let hoverOn: string | null = null
  let focusOn: string | null = null

  let tipFailed = false
  const reaction = (key: string) => message.reactions.find((r) => r.emoji === key)

  // Fetched fresh each time the tip opens: a cache could not tell "one left, one joined" from no
  // change. Only the newest request may fill it; a slow earlier one would show an older list.
  // `keep` refreshes in place (no flash back to "…") when the chip changes under an open tip.
  async function loadTip(key: string, count: number, keep = false) {
    const req = ++tipReq
    tipFor = key
    tipFailed = false
    if (!keep) tipText = '…'
    try {
      const ids = await api.listReactors(message.id, key)
      if (req === tipReq) tipText = tooltipText(ids.map((id) => nameOf(id)), count)
    } catch (err) {
      console.warn('could not load reactors', err)
      if (req === tipReq) {
        tipFailed = true
        tipText = "Couldn't load reactors"
      }
    }
  }
  function showTip(key: string, count: number) {
    if (tipFor === key && !tipFailed) return // already shown or loading
    void loadTip(key, count)
  }
  // The tip stays while the chip is hovered or focused. A click is a hover then a focus, so only the
  // first of them is a "show": a failed tip is retried by the next show, not by its own focus.
  function enter(key: string, count: number, how: 'hover' | 'focus') {
    const engaged = hoverOn === key || focusOn === key
    if (how === 'hover') hoverOn = key
    else focusOn = key
    if (engaged && tipFor === key) return
    showTip(key, count)
  }
  function leave(key: string, how: 'hover' | 'focus') {
    if (how === 'hover') hoverOn = null
    else focusOn = null
    if (tipFor === key && hoverOn !== key && focusOn !== key) {
      tipReq++
      tipFor = null
      // Another chip still has keyboard focus (or the pointer): its tip comes back.
      const back = focusOn ?? hoverOn
      const r = back && reaction(back)
      if (r) showTip(r.emoji, r.count)
    }
  }
  // A click (or someone else's reaction) changes the list under an open tip: refetch it.
  let seen = ''
  $effect(() => {
    const r = tipFor ? reaction(tipFor) : undefined
    const sig = r ? `${r.emoji}|${r.count}|${r.me}` : ''
    untrack(() => {
      const prev = seen
      seen = sig
      if (!r || !prev || prev.split('|')[0] !== r.emoji || prev === sig) return
      void loadTip(r.emoji, r.count, true)
    })
  })
  // Keyed by emoji so it stays put if the chips reorder; ids are alphanumeric, Unicode goes by code point.
  const tipId = (key: string) =>
    `reactors-${message.id}-${key.startsWith('custom:') ? 'custom-' + key.slice(7) : [...key].map((c) => c.codePointAt(0)!.toString(16)).join('-')}`

  function pick(e: Std | CustomEmoji) {
    picking = false
    toggle(reactionKey(e), true)
  }
</script>

{#if !message.deleted && message.reactions.length > 0}
  <div class="reactions">
    {#each message.reactions as r (r.emoji)}
      {@const c = r.emoji.startsWith('custom:') ? customOf(r.emoji) : undefined}
      {@const custom = r.emoji.startsWith('custom:')}
      <span class="wrap">
        <button
          class="chip" class:mine={r.me} type="button" aria-pressed={r.me}
          aria-describedby={tipFor === r.emoji && tipText ? tipId(r.emoji) : undefined}
          onclick={() => toggle(r.emoji, !r.me)}
          onpointerenter={() => enter(r.emoji, r.count, 'hover')}
          onpointerleave={() => leave(r.emoji, 'hover')}
          onfocus={() => enter(r.emoji, r.count, 'focus')}
          onblur={() => leave(r.emoji, 'focus')}
        >
          {#if !custom}
            <img src={twemojiUrl(r.emoji)} alt={r.emoji} draggable="false" />
          {:else if c?.hash}
            <img src={stillSrc(c)} alt={`:${c.name}:`} draggable="false" {@attach animatedEmoji(c)} />
          {:else}
            <span class="gone">:{c?.name ?? r.emoji}:</span>
          {/if}
          <span class="n">{r.count}</span>
        </button>
        {#if tipFor === r.emoji && tipText}<span class="tip" id={tipId(r.emoji)} role="tooltip">{tipText}</span>{/if}
      </span>
    {/each}
    <button bind:this={addBtn} class="chip add" type="button" aria-label="Add reaction" data-menu-toggle onclick={() => (picking = !picking)}>
      <Icon name="smile" size={15} />
    </button>
  </div>
  {#if picking && addBtn}
    <EmojiPopover anchor={addBtn} onPick={pick} onClose={() => (picking = false)} />
  {/if}
{/if}

<style>
  .reactions { display: flex; flex-wrap: wrap; gap: 4px; margin-top: 4px; }
  .wrap { position: relative; display: inline-flex; }
  .chip { display: inline-flex; align-items: center; gap: 5px; height: 24px; padding: 0 8px; border-radius: 8px; border: 1px solid var(--bg-4); background: var(--bg-3); color: var(--text-2); font-size: 13px; }
  .chip:hover { border-color: var(--text-3); }
  .chip.mine { border-color: var(--accent); background: rgba(139, 156, 255, .15); color: var(--text); }
  .chip img { width: 16px; height: 16px; }
  .gone { color: var(--text-3); font-size: 12px; }
  .add { padding: 0 6px; }
  .tip { position: absolute; bottom: calc(100% + 6px); left: 0; z-index: 20; white-space: nowrap; max-width: 320px; overflow: hidden; text-overflow: ellipsis; padding: 4px 8px; border-radius: 6px; background: var(--bg-1); border: 1px solid var(--bg-4); font-size: 12px; color: var(--text); pointer-events: none; }
</style>
