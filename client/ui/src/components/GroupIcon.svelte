<script lang="ts">
  // A group's own icon (#56), round like an avatar. Only drawn when the group has one; without it the callers keep
  // the members' stacked faces. An animated icon shows its still until the profile animation rule says otherwise.
  import { mediaUrl } from '../lib/emoji'
  import { shouldAnimateProfile, wireAnimated } from '../lib/animate.svelte'
  import type { Attachment } from 'svelte/attachments'

  let { hash, still = null, size = 32, hovered }: {
    hash: string
    still?: string | null
    size?: number
    /** Whether the pointer is over the row it sits in (an animated icon plays then). */
    hovered?: () => boolean
  } = $props()

  const animated = $derived(!!still && still !== hash)
  const wire = $derived<Attachment<HTMLImageElement>>((img) => {
    if (animated) return wireAnimated(img, mediaUrl(hash), mediaUrl(still!), (s) => shouldAnimateProfile('list', s), () => !!hovered?.())
  })
</script>

<span class="gicon" style:width="{size}px" style:height="{size}px">
  <img src={mediaUrl(still ?? hash)} alt="" draggable="false" {@attach wire} />
</span>

<style>
  .gicon { flex-shrink: 0; display: inline-grid; border-radius: 50%; overflow: hidden; background: var(--bg-3); }
  .gicon img { width: 100%; height: 100%; object-fit: cover; display: block; }
</style>
