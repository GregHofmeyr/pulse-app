<script lang="ts">
  import { avatarColor, initial } from '../lib/avatar'
  import { app } from '../lib/store.svelte'
  import { userOf } from '../lib/selectors'
  import { mediaUrl } from '../lib/emoji'
  import { shouldAnimateProfile, wireAnimated } from '../lib/animate.svelte'
  import { openCard } from '../lib/profileCard.svelte'
  import type { Attachment } from 'svelte/attachments'

  import type { User } from '../lib/protocol/User'

  let { id, name, size = 30, online = null, place = 'list', interactive = false, serverId = null, hovered, user: shown }: {
    id: string
    name: string
    size?: number
    online?: boolean | null
    place?: 'list' | 'card'
    interactive?: boolean
    serverId?: string | null
    /** In a list: whether the pointer is over this person's row (the avatar animates then too). */
    hovered?: () => boolean
    /** Draw this user instead of the stored one (the profile editor's preview). */
    user?: User
  } = $props()

  // Read from the store so a ProfileUpdated swaps the picture live.
  const user = $derived(shown ?? userOf(app.state, id))
  const hash = $derived(user?.avatar_hash ?? null)
  const still = $derived(user?.avatar_still ?? null)
  let over = $state(false)

  // An animated avatar shows its still until the rule says otherwise; a still-only one never changes.
  const wire = $derived<Attachment<HTMLImageElement>>((img) => {
    if (hash && still && still !== hash) return wireAnimated(img, mediaUrl(hash), mediaUrl(still), (s) => shouldAnimateProfile(place, s), () => over || !!hovered?.())
  })
</script>

{#snippet face()}
  <span class="av" style:width="{size}px" style:height="{size}px" style:font-size="{Math.round(size * 0.42)}px" style:background={hash ? undefined : avatarColor(id)} onpointerenter={() => (over = true)} onpointerleave={() => (over = false)} role="presentation">
    {#if hash}
      <img src={mediaUrl(still ?? hash)} alt="" draggable="false" {@attach wire} />
    {:else}
      {initial(name)}
    {/if}
    {#if online !== null}<span class="pres" class:off={!online} role="img" aria-label={online ? 'online' : 'offline'}></span>{/if}
  </span>
{/snippet}

{#if interactive}
  <button type="button" class="avbtn" aria-label={name} onclick={(e) => openCard(id, serverId, e.currentTarget)}>{@render face()}</button>
{:else}
  {@render face()}
{/if}

<style>
  .av { position: relative; flex-shrink: 0; border-radius: 50%; display: grid; place-items: center; font-weight: 700; color: #fff; }
  .av img { width: 100%; height: 100%; border-radius: 50%; object-fit: cover; display: block; }
  .avbtn { background: none; border: 0; padding: 0; margin: 0; cursor: pointer; display: inline-flex; flex-shrink: 0; }
  .pres { position: absolute; right: -2px; bottom: -2px; width: 34%; height: 34%; min-width: 9px; min-height: 9px; border-radius: 50%; background: var(--ok); border: 2px solid var(--bg-1); box-sizing: border-box; }
  .pres.off { background: #5b5f6b; }
</style>
