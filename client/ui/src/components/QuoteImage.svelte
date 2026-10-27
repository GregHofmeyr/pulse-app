<script lang="ts">
  // An image quote line: its still frame, animated by the custom-emoji rule (lib/animate.svelte.ts).
  // `full`: the full-size view's sizing instead of the in-line one.
  import { animatedEmoji, stillSrc } from '../lib/animate.svelte'
  import type { LineMedia } from '../lib/protocol/LineMedia'

  let { media, caption, full = false }: { media: LineMedia; caption: string | null; full?: boolean } = $props()
</script>

<img class:pic={!full} class:full src={stillSrc(media)} alt={caption ?? 'Quoted image'} draggable="false"
  width={full ? undefined : media.width} height={full ? undefined : media.height} {@attach animatedEmoji(media)} />

<style>
  .pic { display: block; max-width: min(360px, 100%); max-height: 360px; width: auto; height: auto; border-radius: 8px; object-fit: contain; }
  .full { max-width: 92vw; max-height: 92vh; object-fit: contain; border-radius: 8px; }
</style>
