<script lang="ts">
  // A custom status's emoji (#152): a standard one as Twemoji, or a server's custom emoji (animated on hover, like a
  // reaction). A custom one since deleted shows nothing; the text still reads.
  import { app } from '../lib/store.svelte'
  import { twemojiUrl } from '../lib/emoji'
  import { animatedEmoji, stillSrc } from '../lib/animate.svelte'

  let { emoji, size = 16 }: { emoji: string; size?: number } = $props()
  const custom = $derived(emoji.startsWith('custom:') ? app.state.customEmoji[emoji.slice(7)] : undefined)
</script>

{#if !emoji.startsWith('custom:')}
  <img class="status-emoji" src={twemojiUrl(emoji)} alt={emoji} draggable="false" style:width="{size}px" style:height="{size}px" />
{:else if custom?.hash}
  <img class="status-emoji" src={stillSrc(custom)} alt=":{custom.name}:" draggable="false" style:width="{size}px" style:height="{size}px" {@attach animatedEmoji(custom)} />
{/if}

<style>
  .status-emoji { flex-shrink: 0; object-fit: contain; vertical-align: middle; }
</style>
