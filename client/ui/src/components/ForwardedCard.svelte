<script lang="ts">
  // A forwarded message (#302): under the forwarder's comment, a card that says where the original was said ("Forwarded
  // from #general in Main", or just "a DM" / "a group"), who said it and when, then what it said: its text as it was
  // forwarded, its GIF, its files (the original's, by reference) and its link previews. The header opens the original
  // when it's somewhere you can see; one deleted shows only that it's gone.
  import { canOpenOriginal, forwardedFrom } from '../lib/forward'
  import { gifOnly, gifsOf } from '../lib/giphy'
  import { renderMessage } from '../lib/markdown'
  import { openMessageAt } from '../lib/reminders.svelte'
  import { previews } from '../lib/linkPreviews.svelte'
  import { app, ui } from '../lib/store.svelte'
  import type { Message } from '../lib/protocol/Message'
  import Icon from './Icon.svelte'
  import GifView from './GifView.svelte'
  import LinkEmbed from './LinkEmbed.svelte'
  import MessageFiles from './MessageFiles.svelte'

  let { message, nameOf, mentionNames = [], onError = () => {} }: {
    /** The forward (its `forwarded` set); its attachments are the original's. */
    message: Message
    nameOf: (userId: string | null) => string
    mentionNames?: string[]
    onError?: (text: string) => void
  } = $props()

  const f = $derived(message.forwarded!)
  const from = $derived(forwardedFrom(app.state, f))
  const openable = $derived(canOpenOriginal(app.state, f))
  const when = $derived(new Date(f.created_at).toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' }))
  // the original as a message of its own, for the shared renderers (its own id keeps its render cache apart)
  const original = $derived<Message>({ ...message, id: `${message.id}:fwd`, content: f.content, gif: f.gif, edited_at: null, embeds: f.embeds ?? [], forwarded: undefined, caption_is_fallback: false })
  const html = $derived(f.content ? renderMessage(original, mentionNames, app.state.customEmoji) : '')
  let failedGifs = $state<string[]>([])
  const gifs = $derived(f.gif || previews.show ? gifsOf(original).filter((g) => !failedGifs.includes(g.id)) : [])
  const onlyGif = $derived(gifs.length > 0 && gifOnly(original))
  const embeds = $derived(previews.show ? (f.embeds ?? []) : [])

  function open() {
    if (!openable) return void (ui.notice = f.unavailable ? 'The original message is no longer available.' : "You can't see where the original was sent.")
    void openMessageAt(f.channel_id, f.message_id, f.thread_root_id)
  }
</script>

<div class="fwd" class:gone={f.unavailable}>
  <button type="button" class="head" onclick={open} aria-disabled={!openable}
    data-tip={openable ? 'Open the original' : undefined}>
    <Icon name="reply" size={12} />
    <span class="from">Forwarded from {from}</span>
    {#if !f.unavailable}<span class="by">· {nameOf(f.author_id)} · <time datetime={f.created_at}>{when}</time></span>{/if}
  </button>
  {#if f.unavailable}
    <p class="unavailable">The original message is no longer available.</p>
  {:else}
    {#if html && !onlyGif}
      <!-- renderMessage output is DOMPurify-sanitised (allow-list, cached per message) -->
      <div class="content">{@html html}</div>
    {/if}
    {#each gifs as g (g.id)}<GifView gif={g} {onError} onFail={() => (failedGifs = [...failedGifs, g.id])} />{/each}
    {#if message.attachments?.length}<MessageFiles {message} {onError} />{/if}
    {#each embeds as e (e.url)}<LinkEmbed embed={e} messageId={message.id} mine={false} {onError} />{/each}
  {/if}
</div>

<style>
  .fwd { margin-top: 4px; padding: 6px 10px 8px; border-left: 3px solid var(--bg-4); border-radius: var(--radius-xs); background: var(--bg-inset); display: flex; flex-direction: column; gap: 4px; min-width: 0; }
  .head { display: flex; align-items: center; gap: 6px; padding: 0; border: 0; background: none; color: var(--text-3); font: inherit; font-size: 12px; text-align: left; cursor: pointer; min-width: 0; }
  .head[aria-disabled='true'] { cursor: default; }
  .head:not([aria-disabled='true']):hover .from, .head:focus-visible .from { color: var(--text); text-decoration: underline; }
  .head :global(svg) { transform: scaleX(-1); flex-shrink: 0; }
  .from { font-weight: 600; color: var(--text-2); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .by { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .content { color: var(--text-body); line-height: 1.5; overflow-wrap: anywhere; }
  .content :global(p) { margin: 0; }
  .content :global(a) { color: var(--accent-text); }
  .content :global(code) { font-family: 'JetBrains Mono', ui-monospace, monospace; font-size: 12.5px; background: var(--bg-3); border-radius: var(--radius-xs); padding: 1px 6px; }
  .content :global(pre) { background: var(--bg-3); border-radius: var(--radius-sm); padding: 10px; overflow-x: auto; }
  .content :global(img.emoji) { width: 1.375em; height: 1.375em; vertical-align: -0.3em; object-fit: contain; }
  .unavailable { margin: 0; font-style: italic; color: var(--text-3); font-size: 13px; }
</style>
