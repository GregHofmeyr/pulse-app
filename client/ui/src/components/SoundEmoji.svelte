<script lang="ts">
  // A sound's emoji wherever sounds show: its custom emoji (animated on hover like chat's), its Unicode emoji, or the
  // soundboard icon when it has neither or its custom emoji has since been deleted.
  import { app } from '../lib/store.svelte'
  import { animatedEmoji, stillSrc } from '../lib/animate.svelte'
  import Icon from './Icon.svelte'

  let { emoji, emojiId, size = 22 }: { emoji: string | null; emojiId: string | null; size?: number } = $props()

  const custom = $derived(emojiId ? app.state.customEmoji[emojiId] : undefined)
  const live = $derived(custom && custom.hash !== null ? custom : null)
</script>

{#if live}
  <img class="sound-emoji" src={stillSrc(live)} alt={`:${live.name}:`} width={size} height={size} draggable="false" {@attach animatedEmoji(live)} />
{:else if emoji}
  <span class="sound-emoji" style:font-size="{size}px">{emoji}</span>
{:else}
  <Icon name="soundboard" {size} />
{/if}

<style>
  img.sound-emoji { object-fit: contain; display: block; }
  span.sound-emoji { line-height: 1; }
</style>
