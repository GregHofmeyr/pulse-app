<script lang="ts">
  // Someone's custom status in one muted line (#152): its emoji, then its text, cut with an ellipsis; the whole text
  // in the tooltip. Plain text only (Svelte escapes it): never markdown, links or mentions.
  import StatusEmoji from './StatusEmoji.svelte'
  import type { CustomStatus } from '../lib/protocol/CustomStatus'

  let { status, size = 14 }: { status: CustomStatus; size?: number } = $props()
</script>

<span class="custom-status" data-tip={status.text || undefined}>
  {#if status.emoji}<StatusEmoji emoji={status.emoji} {size} />{/if}{#if status.text}<span class="text">{status.text}</span>{/if}
</span>

<style>
  .custom-status { display: inline-flex; align-items: center; gap: 4px; min-width: 0; max-width: 100%; color: var(--text-3); font-size: 12px; font-weight: 400; }
  .text { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
