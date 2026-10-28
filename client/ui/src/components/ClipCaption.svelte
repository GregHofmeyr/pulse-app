<script lang="ts">
  // A clip's caption (its message content), rendered like message text. @mentions aren't resolved: a caption never pings.
  import { renderMessage } from '../lib/markdown'
  import { app } from '../lib/store.svelte'
  import type { Message } from '../lib/protocol/Message'

  let { message }: { message: Message } = $props()

  const NO_MENTIONS: string[] = []
</script>

{#if message.content.trim()}
  <!-- renderMessage output is DOMPurify-sanitised (allow-list, cached per message), as in MessageItem. -->
  <div class="caption">{@html renderMessage(message, NO_MENTIONS, app.state.customEmoji)}</div>
{/if}

<style>
  .caption { color: #dcdee5; line-height: 1.45; overflow-wrap: anywhere; }
  .caption :global(p) { margin: 0; }
  .caption :global(a) { color: var(--accent); }
  .caption :global(code) { font-family: 'JetBrains Mono', ui-monospace, monospace; font-size: 12.5px; background: #17181c; border: 1px solid var(--line); border-radius: 6px; padding: 1px 6px; }
  .caption :global(img.emoji) { width: 1.375em; height: 1.375em; vertical-align: -0.3em; object-fit: contain; }
</style>
