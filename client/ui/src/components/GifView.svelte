<script lang="ts">
  // A GIF in chat (#58): a picked one or a pasted GIPHY link, drawn from GIPHY's media links for its id (nothing of
  // GIPHY's is stored). It plays while the window is focused and it's on screen; under reduced motion it shows its
  // still until pointed at or clicked (lib/animate.svelte.ts shouldPlayGif). A star on hover keeps it in your
  // favourites.
  import Icon from './Icon.svelte'
  import { animatedGif, motion } from '../lib/animate.svelte'
  import { chatSize, gifMedia, type ShownGif } from '../lib/giphy'
  import { isGifFav, loadGifFavs, toggleGifFav } from '../lib/gifs.svelte'
  import { app } from '../lib/store.svelte'
  import { errorText } from '../lib/tauri'

  let { gif, onError = () => {} }: { gif: ShownGif; onError?: (text: string) => void } = $props()

  const media = $derived(gifMedia(gif.id))
  const size = $derived(chatSize(gif))
  const label = $derived(gif.title || 'GIF')
  // a click pauses it, or under reduced motion plays it
  let toggled = $state(false)
  // a pasted link's size, from its still once loaded (for starring it)
  let natural = $state<{ width: number; height: number } | null>(null)
  const starred = $derived(isGifFav(gif.id))

  function star() {
    const me = app.state.me?.id
    if (!me) return
    const width = gif.width ?? natural?.width ?? 200
    const height = gif.height ?? natural?.height ?? 200
    void loadGifFavs(me)
      .then(() => toggleGifFav({ id: gif.id, title: gif.title, width, height }))
      .catch((e) => onError(errorText(e)))
  }
  function seen(img: HTMLImageElement) {
    if (!natural && img.naturalWidth && img.naturalHeight) natural = { width: img.naturalWidth, height: img.naturalHeight }
  }
  // whether a favourite shows starred: fetched once per sign-in, when the first GIF shows
  $effect(() => {
    const me = app.state.me?.id
    if (me) void loadGifFavs(me)
  })
</script>

<div class="gif" class:sized={!!size} style:width={size ? `${size.width}px` : null} style:height={size ? `${size.height}px` : null}>
  <button class="play" type="button" aria-label={label} aria-pressed={motion.reducedMotion ? toggled : !toggled} onclick={() => (toggled = !toggled)}>
    <img src={media.still} alt={label} width={size?.width} height={size?.height} draggable="false"
      onload={(e) => seen(e.currentTarget as HTMLImageElement)} {@attach animatedGif(media.anim, media.still, () => toggled)} />
  </button>
  <button class="icon-btn icon-btn-sm icon-btn-overlay star" class:on={starred} type="button"
    aria-label={starred ? 'Remove from favourite GIFs' : 'Add to favourite GIFs'} aria-pressed={starred} data-tip={starred ? 'Unfavourite' : 'Favourite'}
    onclick={star}><Icon name="star" size={16} /></button>
</div>

<style>
  /* GIPHY's 200 px high rendition, at most 300 px wide (lib/giphy.ts chatSize); a pasted link's shape is unknown, so
     it sizes to its image within the same box */
  .gif { position: relative; display: block; width: fit-content; max-width: min(300px, 100%); margin-top: 4px; border-radius: var(--radius-sm); overflow: hidden; background: var(--bg-3); }
  .play { display: block; padding: 0; border: 0; background: none; cursor: pointer; }
  .play img { display: block; height: 200px; width: auto; max-width: min(300px, 100%); object-fit: cover; }
  .gif.sized .play, .gif.sized .play img { width: 100%; height: 100%; max-width: none; }
  .star { position: absolute; top: 6px; right: 6px; opacity: 0; }
  .gif:hover .star, .star:focus-visible { opacity: 1; }
  /* starred: the star filled */
  .star.on :global(svg) { fill: currentColor; }
</style>
