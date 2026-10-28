<script lang="ts">
  // A clip's "View N replies" link (with a "new" marker) and its unsent-reply marker; both open its replies panel.
  import Icon from './Icon.svelte'
  import { hasNewClipReplies } from '../lib/clips'
  import type { Message } from '../lib/protocol/Message'

  let { message, onOpenReplies, unsentReplies = 0 }: {
    message: Message
    onOpenReplies: () => void
    /** Your replies to it that failed to send, shown while its panel is closed. */
    unsentReplies?: number
  } = $props()

  const count = $derived(message.clip?.reply_count ?? 0)
</script>

{#if count > 0}
  <button type="button" class="replies" onclick={onOpenReplies}>
    View {count} {count === 1 ? 'reply' : 'replies'}
    {#if hasNewClipReplies(message)}<span class="new">new</span>{/if}
  </button>
{/if}
{#if unsentReplies > 0}
  <button type="button" class="unsent-reply" onclick={onOpenReplies}>
    <Icon name="reply" size={13} />{unsentReplies === 1 ? 'Unsent reply' : `${unsentReplies} unsent replies`}
  </button>
{/if}

<style>
  .replies { align-self: flex-start; padding: 0; border: 0; background: none; color: var(--accent); font-size: 13px; display: inline-flex; align-items: center; gap: 6px; }
  .replies:hover { text-decoration: underline; }
  .new { padding: 0 6px; border-radius: 8px; background: var(--accent); color: var(--on-accent); font-size: 10px; font-weight: 700; text-transform: uppercase; }
  .unsent-reply { align-self: flex-start; padding: 0; border: 0; background: none; color: var(--danger); font-size: 13px; display: inline-flex; align-items: center; gap: 5px; }
  .unsent-reply:hover { text-decoration: underline; }
</style>
