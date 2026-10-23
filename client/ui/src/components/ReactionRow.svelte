<script lang="ts">
  import type { Message } from '../lib/protocol/Message'
  import type { CustomEmoji } from '../lib/protocol/CustomEmoji'
  import type { Std } from '../lib/emoji'
  import { mediaUrl, reactionKey, twemojiUrl } from '../lib/emoji'
  import { clickOutside } from '../lib/actions'
  import { memberServers, popoverStyle, react, tooltipText } from '../lib/reactions'
  import { app } from '../lib/store.svelte'
  import { api } from '../lib/tauri'
  import EmojiPicker from './EmojiPicker.svelte'
  import Icon from './Icon.svelte'

  let { message, meId, nameOf, onError = () => {} }: {
    message: Message
    meId: string
    nameOf: (userId: string | null) => string
    /** Where a failed add/remove is reported (the channel's error line). */
    onError?: (msg: string) => void
  } = $props()

  // Reactors per message+emoji; the key carries the count and `me`, so the next event for it drops the entry.
  const cache = new Map<string, string[]>()
  let tipFor = $state<string | null>(null)
  let tipText = $state('')
  let animated = $state<string | null>(null)
  let picking = $state<string | null>(null)
  let addBtn: HTMLButtonElement | undefined = $state()

  const customOf = (key: string): CustomEmoji | undefined => app.state.customEmoji[key.slice(7)]

  function toggle(key: string, add: boolean) {
    void react(() => app.state, (s) => (app.state = s), message.channel_id, message.id, key, add, meId, onError)
  }

  async function hover(key: string, count: number, me: boolean) {
    tipFor = key
    const ck = `${message.id}|${key}|${count}|${me}`
    let ids = cache.get(ck)
    if (!ids) {
      tipText = '…'
      try {
        ids = await api.listReactors(message.id, key)
        cache.set(ck, ids)
      } catch {
        if (tipFor === key) tipText = ''
        return
      }
    }
    if (tipFor === key) tipText = tooltipText(ids.map((id) => nameOf(id)), count)
  }

  function pick(e: Std | CustomEmoji) {
    picking = null
    toggle(reactionKey(e), true)
  }

  const style = () =>
    addBtn
      ? popoverStyle(addBtn.getBoundingClientRect(), { width: 360, height: 420 }, { width: window.innerWidth, height: window.innerHeight })
      : ''
</script>

{#if !message.deleted && message.reactions.length > 0}
  <div class="reactions">
    {#each message.reactions as r (r.emoji)}
      {@const c = r.emoji.startsWith('custom:') ? customOf(r.emoji) : undefined}
      {@const custom = r.emoji.startsWith('custom:')}
      <span class="wrap">
        <button
          class="chip" class:mine={r.me} type="button" aria-pressed={r.me}
          onclick={() => toggle(r.emoji, !r.me)}
          onpointerenter={() => { animated = r.emoji; void hover(r.emoji, r.count, r.me) }}
          onpointerleave={() => { animated = null; tipFor = null }}
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
        {#if tipFor === r.emoji && tipText}<span class="tip" role="tooltip">{tipText}</span>{/if}
      </span>
    {/each}
    <button bind:this={addBtn} class="chip add" type="button" aria-label="Add reaction" data-menu-toggle onclick={() => (picking = picking ? null : 'row')}>
      <Icon name="smile" size={15} />
    </button>
  </div>
  {#if picking}
    <div use:clickOutside={() => (picking = null)} style={style()}>
      <EmojiPicker onPick={pick} onClose={() => (picking = null)} servers={app.state.servers} memberOf={memberServers(app.state)} />
    </div>
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
