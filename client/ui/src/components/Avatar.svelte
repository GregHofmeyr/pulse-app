<script lang="ts">
  import { avatarColor, initial } from '../lib/avatar'
  import { app } from '../lib/store.svelte'
  import { userOf } from '../lib/selectors'
  import { mediaUrl } from '../lib/emoji'
  import { shouldAnimateProfile, wireAnimated } from '../lib/animate.svelte'
  import { openCard } from '../lib/profileCard.svelte'
  import type { Attachment } from 'svelte/attachments'

  import type { User } from '../lib/protocol/User'
  import type { Presence } from '../lib/presence'

  let { id, name, size = 30, presence = null, place = 'list', interactive = false, serverId = null, hovered, user: shown }: {
    id: string
    name: string
    size?: number
    /** The dot to draw (lib/presence.ts presenceOf), or none. */
    presence?: Presence | null
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
  const animated = $derived(!!hash && !!still && still !== hash)
  let over = $state(false)

  // An animated avatar shows its still until the rule says otherwise; a still-only one never changes.
  const wire = $derived<Attachment<HTMLImageElement>>((img) => {
    if (animated) return wireAnimated(img, mediaUrl(hash!), mediaUrl(still!), (s) => shouldAnimateProfile(place, s), () => over || !!hovered?.())
  })
  // Only an animated avatar watches the pointer (it plays while hovered). A touch tap ends with the touch.
  const track: Attachment<HTMLElement> = (el) => {
    const enter = () => (over = true)
    const leave = () => (over = false)
    const up = (e: PointerEvent) => {
      if (e.pointerType === 'touch') over = false
    }
    el.addEventListener('pointerenter', enter)
    el.addEventListener('pointerleave', leave)
    el.addEventListener('pointerup', up)
    el.addEventListener('pointercancel', leave)
    return () => {
      el.removeEventListener('pointerenter', enter)
      el.removeEventListener('pointerleave', leave)
      el.removeEventListener('pointerup', up)
      el.removeEventListener('pointercancel', leave)
      over = false
    }
  }
</script>

{#snippet face()}
  <span class="av" style:width="{size}px" style:height="{size}px" style:font-size="{Math.round(size * 0.42)}px" style:background={hash ? undefined : avatarColor(id)} {@attach animated && track}>
    {#if hash}
      <img src={mediaUrl(still ?? hash)} alt="" draggable="false" {@attach wire} />
    {:else}
      {initial(name)}
    {/if}
    {#if presence}<span class="pres {presence.kind}" role="img" aria-label={presence.label}></span>{/if}
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
  .pres { position: absolute; right: -2px; bottom: -2px; width: 34%; height: 34%; min-width: 9px; min-height: 9px; border-radius: 50%; background: var(--ok); border: 2px solid var(--ring); box-sizing: border-box; }
  /* your own Invisible looks like offline (#171): the status picker and the line under your name say Invisible */
  .pres.offline, .pres.invisible { background: var(--presence-off); }
  /* Do Not Disturb (#45): red, with a bar across it in the cut-out colour, as on Discord */
  .pres.dnd { background: var(--danger); }
  .pres.dnd::after { content: ''; position: absolute; left: 18%; right: 18%; top: 50%; height: 26%; transform: translateY(-50%); border-radius: 999px; background: var(--ring); }
</style>
