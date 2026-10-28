<script lang="ts">
  // The quick card next to the name or avatar that opened it (profileUi.card). The person comes from
  // the store and the card from the profile cache, so a ProfileUpdated restyles an open card without
  // a second fetch; a cache cleared on reconnect refetches.
  import { untrack } from 'svelte'
  import ProfileCard from './ProfileCard.svelte'
  import { app, loadProfile } from '../lib/store.svelte'
  import { userOf } from '../lib/selectors'
  import { DEFAULT_THEME } from '../lib/cardTheme'
  import { errorText } from '../lib/tauri'
  import { closeCard, openFull, profileUi } from '../lib/profileCard.svelte'
  import type { CardProfile } from '../lib/protocol/CardProfile'

  const WIDTH = 300
  const GAP = 8

  const target = $derived(profileUi.card)
  const cached = $derived(target ? app.state.profiles[target.userId] : undefined)
  const user = $derived(target ? (userOf(app.state, target.userId) ?? cached?.user ?? null) : null)
  const PLACEHOLDER: CardProfile = { banner_hash: null, banner_still: null, theme_top: DEFAULT_THEME.top, theme_bottom: DEFAULT_THEME.bottom, pronouns: null, bio: null }
  let failed = $state<string | null>(null)
  /** The card's measured height: it grows when the skeleton becomes the loaded card. */
  let height = $state(0)

  // Beside the anchor: to its right, or its left when that doesn't fit; kept inside the window.
  const pos = $derived.by(() => {
    if (!target) return { left: 0, top: 0 }
    const r = target.anchor
    const vw = window.innerWidth
    const vh = window.innerHeight
    const left = r.right + GAP + WIDTH <= vw - GAP ? r.right + GAP : Math.max(GAP, r.left - GAP - WIDTH)
    const top = Math.max(GAP, Math.min(r.top, vh - height - GAP))
    return { left: Math.round(left), top: Math.round(top) }
  })

  // Fetch when there is nothing cached (shared with any load already in flight).
  $effect(() => {
    const id = target?.userId
    if (!id || cached) return
    untrack(() => {
      failed = null
      loadProfile(id).catch((e) => {
        if (profileUi.card?.userId === id) failed = errorText(e)
      })
    })
  })

  let pop = $state<HTMLElement>()
  let returnTo: HTMLElement | null = null
  let shown: HTMLElement | null = null // the card's element, still known while it is being removed
  // On open (the attachment below): remember what had focus, the name or avatar that opened it, and
  // focus the card. On close, however it closes: give focus back, unless it has moved elsewhere meanwhile.
  function opened(el: HTMLElement) {
    const a = document.activeElement
    if (!returnTo && a instanceof HTMLElement && a !== document.body && !el.contains(a)) returnTo = a
    shown = el
    el.focus()
    const measure = () => (height = el.offsetHeight)
    measure()
    const ro = typeof ResizeObserver === 'function' ? new ResizeObserver(measure) : null
    ro?.observe(el)
    return () => ro?.disconnect()
  }
  // Re-measure once the loaded card replaces the skeleton (where there is no ResizeObserver).
  $effect(() => {
    void cached
    if (shown) height = shown.offsetHeight
  })
  $effect(() => {
    if (target) return
    untrack(() => {
      const el = returnTo
      returnTo = null
      const a = document.activeElement
      if (el?.isConnected && (!a || a === document.body || shown?.contains(a))) el.focus()
      shown = null
    })
  })

  function onKey(e: KeyboardEvent) {
    if (!target || e.key !== 'Escape' || e.defaultPrevented) return
    e.preventDefault()
    closeCard()
  }
  function onDown(e: PointerEvent | Event) {
    const t = e.target as Node
    // a press on the name or avatar that opened it is left to its click, which closes it (openCard toggles)
    if (!target || !pop || pop.contains(t) || returnTo?.contains(t) || onAnchor(e, target.anchor)) return
    closeCard()
  }
  // The anchor may not have taken focus on click (WebKit doesn't focus buttons): its rectangle decides then.
  const onAnchor = (e: Event, r: DOMRect) => e instanceof MouseEvent && e.clientX >= r.left && e.clientX <= r.right && e.clientY >= r.top && e.clientY <= r.bottom
  function viewFull() {
    if (!target) return
    const { userId, serverId } = target
    closeCard()
    openFull(userId, serverId)
  }
</script>

<!-- the position is a snapshot of the anchor: a resize closes rather than leaving it adrift -->
<svelte:window onkeydown={onKey} onpointerdowncapture={onDown} onresize={() => target && closeCard()} />

{#if target && user}
  <div bind:this={pop} class="pop" role="dialog" aria-label="Profile of {user.username}" tabindex="-1" {@attach opened} style:left="{pos.left}px" style:top="{pos.top}px">
    <ProfileCard {user} card={cached?.card ?? PLACEHOLDER} badges={cached?.badges ?? []} stats={cached?.stats ?? null} serverId={target.serverId}
      loading={!cached && !failed} onViewFull={viewFull} onClose={closeCard} />
    {#if failed && !cached}<p class="err" role="alert">{failed}</p>{/if}
  </div>
{/if}

<style>
  .pop { position: fixed; z-index: 40; outline: none; max-height: calc(100vh - 16px); overflow: auto; }
  .err { margin: 6px 0 0; padding: 6px 10px; border-radius: 8px; background: var(--bg-3); color: var(--text-2); font-size: 12px; max-width: 300px; }
</style>
