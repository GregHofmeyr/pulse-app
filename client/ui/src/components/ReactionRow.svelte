<script lang="ts">
  import type { Message } from '../lib/protocol/Message'
  import type { CustomEmoji } from '../lib/protocol/CustomEmoji'
  import type { Std } from '../lib/emoji'
  import { mediaUrl, reactionKey, twemojiUrl } from '../lib/emoji'
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
  let animated = $state<string | null>(null)
  let picking = $state(false)
  let addBtn: HTMLButtonElement | undefined = $state()

  const customOf = (key: string): CustomEmoji | undefined => app.state.customEmoji[key.slice(7)]

  function toggle(key: string, add: boolean) {
    void react(() => app.state, (s) => (app.state = s), message.channel_id, message.id, key, add, meId, onError)
  }

  let tipReq = 0

  // Fetched fresh on every hover/focus: a cache could not tell "one left, one joined" from no change.
  // Only the newest request may fill the tooltip; a slow earlier one would show an older list.
  async function showTip(key: string, count: number) {
    const req = ++tipReq
    tipFor = key
    tipText = '…'
    try {
      const ids = await api.listReactors(message.id, key)
      if (req === tipReq) tipText = tooltipText(ids.map((id) => nameOf(id)), count)
    } catch {
      if (req === tipReq) tipText = ''
    }
  }
  function hideTip() {
    tipReq++
    tipFor = null
  }
  const tipId = (i: number) => `reactors-${message.id}-${i}`

  function pick(e: Std | CustomEmoji) {
    picking = false
    toggle(reactionKey(e), true)
  }
</script>

{#if !message.deleted && message.reactions.length > 0}
  <div class="reactions">
    {#each message.reactions as r, i (r.emoji)}
      {@const c = r.emoji.startsWith('custom:') ? customOf(r.emoji) : undefined}
      {@const custom = r.emoji.startsWith('custom:')}
      <span class="wrap">
        <button
          class="chip" class:mine={r.me} type="button" aria-pressed={r.me}
          aria-describedby={tipFor === r.emoji && tipText ? tipId(i) : undefined}
          onclick={() => toggle(r.emoji, !r.me)}
          onpointerenter={() => { animated = r.emoji; void showTip(r.emoji, r.count) }}
          onpointerleave={() => { animated = null; hideTip() }}
          onfocus={() => void showTip(r.emoji, r.count)}
          onblur={hideTip}
        >
          {#if !custom}
            <img src={twemojiUrl(r.emoji)} alt={r.emoji} draggable="false" />
          {:else if c?.hash}
            <img src={mediaUrl((animated === r.emoji ? c.hash : (c.still_hash ?? c.hash)))} alt={`:${c.name}:`} draggable="false" />
          {:else}
            <span class="gone">:{c?.name ?? r.emoji}:</span>
          {/if}
          <span class="n">{r.count}</span>
        </button>
        {#if tipFor === r.emoji && tipText}<span class="tip" id={tipId(i)} role="tooltip">{tipText}</span>{/if}
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
