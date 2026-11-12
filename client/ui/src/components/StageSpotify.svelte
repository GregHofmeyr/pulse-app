<script lang="ts">
  // A Spotify link shared to the voice stage (#234), as you chose to see it (lib/stageShareView): the strip above
  // the tiles, or a pill in the stage's bottom-right corner that grows into the corner card (#288). Hidden, it isn't
  // here at all: the header's Spotify button brings it back.
  //
  // Spotify's rules as on the #196 card: their single-colour icon, never over the artwork; the artwork as they give
  // it (uncropped, 4px corners, nothing on top); their approved link-out wording; Pulse never controls anyone's Spotify.
  import { tick } from 'svelte'
  import Icon from './Icon.svelte'
  import SpotifyIcon from './SpotifyIcon.svelte'
  import { isMediaHash, mediaUrl } from '../lib/emoji'
  import { growFromCorner } from '../lib/flip'
  import { closeTimer, PILL_CLOSE_GRACE, PILL_OPEN_DELAY } from '../lib/hoverIntent'
  import { serverNow } from '../lib/serverClock'
  import { mayStop, stageCard } from '../lib/stageShare'
  import { modeFor, setMode, type StageMode } from '../lib/stageShareView.svelte'
  import { nameOfIn } from '../lib/selectors'
  import { app, clock } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import type { StageShare } from '../lib/protocol/StageShare'

  let { share, serverId, inCall, place }: {
    share: StageShare
    serverId: string | null
    /** You're in this call: Stop sharing is yours to use. */
    inCall: boolean
    /** Which part this instance draws: the strip above the tiles, or the floating pill / corner card. */
    place: 'top' | 'corner'
  } = $props()

  const nameOf = $derived(nameOfIn(() => app.state, serverId))
  // the clock moves every 30 s, which is plenty for "may have ended" (by the server's clock, #258)
  const now = $derived((void clock.nowIso, serverNow()))
  const card = $derived(stageCard(share, nameOf(share.shared_by), now))
  const mode = $derived(modeFor(share.id))
  const canStop = $derived(mayStop(share, app.state.me?.id ?? '', inCall))
  // a Jam's "Now playing" (#275) shows its song's art, when the server found one; otherwise the link's own
  const thumb = $derived(card.nowPlaying && share.now_playing?.art && isMediaHash(share.now_playing.art.hash) ? share.now_playing.art : share.embed.thumb)
  const art = $derived(thumb && isMediaHash(thumb.hash) ? mediaUrl(thumb.hash) : null)

  let error = $state('')
  async function stop() {
    error = ''
    try {
      await api.stopStageShare(share.channel_id, share.id)
    } catch (e) {
      error = errorText(e)
    }
  }
  // Each view change unmounts the control you used: the focus goes on to where that view is now (the header's
  // Spotify button once it's hidden), never down to the page.
  const FOCUS: Record<StageMode, string> = {
    strip: '.sp-strip button[aria-label="Minimise"]',
    pill: '.stage-corner .pill',
    hidden: '[data-stage-spotify]',
  }
  async function show(m: StageMode) {
    const had = !!(document.activeElement && document.activeElement !== document.body)
    setMode(share.id, m)
    await tick()
    if (had && (!document.activeElement || document.activeElement === document.body)) document.querySelector<HTMLElement>(FOCUS[m])?.focus()
  }

  // The pill grows into the corner card (#288) and shrinks back: one element, so the pointer never crosses a gap.
  // Opened by a hover that means it, it closes once the pointer has been gone a moment; opened on purpose (a click,
  // Enter or Space), it stays until a click outside, Escape, or the focus leaving it. Never on focus alone, and never
  // on a pointer that's just there without moving (a window coming back to the front sends that, #288's bug).
  let open = $state<false | 'hover' | 'pinned'>(false)
  let zone = $state<HTMLDivElement>()
  let pillSize: { width: number; height: number } | null = null
  let anim: Animation | null = null
  let closing = false
  // where the pointer came in: a hover counts once it has moved from there
  let entered: { x: number; y: number } | null = null
  let opening = false
  const openTimer = closeTimer(PILL_OPEN_DELAY)
  const closeLater = closeTimer(PILL_CLOSE_GRACE)
  const cardEl = () => zone?.querySelector<HTMLElement>('.corner-card') ?? null
  const pillEl = () => zone?.querySelector<HTMLElement>('.pill') ?? null

  async function grow(how: 'hover' | 'pinned', focusIn: boolean) {
    openTimer.cancel()
    opening = false
    closeLater.cancel()
    if (open && !closing) {
      if (how === 'pinned') open = 'pinned'
      return
    }
    if (closing) {
      // coming back while it shrinks: it stays the card
      anim?.cancel()
      anim = null
      closing = false
      open = how
      return
    }
    const pill = pillEl()
    const r = pill?.getBoundingClientRect()
    if (r?.width) pillSize = { width: r.width, height: r.height }
    const hadFocus = !!pill && pill === document.activeElement
    open = how
    await tick()
    const c = cardEl()
    if (!c) return
    if (pillSize) anim = growFromCorner(c, pillSize)
    if (focusIn || hadFocus) c.querySelector<HTMLElement>('button, a')?.focus()
  }

  function shrink(focusPill = false) {
    openTimer.cancel()
    opening = false
    closeLater.cancel()
    if (!open || closing) return
    const done = async () => {
      anim = null
      closing = false
      open = false
      if (focusPill) {
        await tick()
        pillEl()?.focus()
      }
    }
    const c = cardEl()
    const a = c && pillSize ? growFromCorner(c, pillSize, true) : null
    if (!a) return void done()
    anim?.cancel()
    anim = a
    closing = true
    a.onfinish = () => void done()
  }

  function pointerEnter(e: PointerEvent) {
    closeLater.cancel()
    entered = { x: e.clientX, y: e.clientY }
  }
  function pointerMove(e: PointerEvent) {
    if (open || opening || !entered) return
    if (e.clientX === entered.x && e.clientY === entered.y) return
    opening = true
    openTimer.schedule(() => void grow('hover', false))
  }
  function pointerLeave() {
    entered = null
    openTimer.cancel()
    opening = false
    if (open === 'hover') closeLater.schedule(() => shrink())
  }
  function keydown(e: KeyboardEvent) {
    if (e.key !== 'Escape' || !open) return
    e.preventDefault()
    e.stopPropagation()
    entered = null
    shrink(true)
  }
  function focusOut(e: FocusEvent) {
    const to = e.relatedTarget as Node | null
    // null: the window lost focus, or a press on nothing focusable (the press outside closes it)
    if (open && to && !zone?.contains(to)) shrink()
  }
  $effect(() => {
    if (!open) return
    const down = (e: PointerEvent) => {
      if (!zone?.contains(e.target as Node)) {
        entered = null
        shrink()
      }
    }
    window.addEventListener('pointerdown', down, true)
    return () => window.removeEventListener('pointerdown', down, true)
  })
  $effect(() => {
    if (mode !== 'pill') {
      open = false
      closing = false
      entered = null
    }
  })
  $effect(() => () => {
    openTimer.cancel()
    closeLater.cancel()
    anim?.cancel()
  })
</script>

{#snippet artwork(size: 'sm' | 'md' | 'lg')}
  <!-- Spotify's artwork as they give it: uncropped, 4px corners, nothing over it -->
  <span class="art {size}" class:tile={!art} aria-hidden="true">
    {#if art}<img src={art} alt="" width={thumb!.width} height={thumb!.height} />{:else}<Icon name="headphones" size={size === 'sm' ? 14 : 18} />{/if}
  </span>
{/snippet}

{#snippet words()}
  <span class="words">
    <span class="title" data-tip={card.title}>{card.title}</span>
    <span class="sub" data-tip={card.subtitle}>{card.subtitle}</span>
    {#if card.nowPlaying}<span class="sub now" data-tip={card.nowPlaying}>{card.nowPlaying}</span>{/if}
  </span>
{/snippet}

{#snippet openLink()}
  <a class="btn {card.jam ? 'btn-primary' : 'btn-secondary'} sp-open" href={share.embed.url} target="_blank" rel="noopener noreferrer">{card.action}</a>
{/snippet}

{#snippet stopBtn()}
  {#if canStop}<button type="button" class="btn btn-quiet sp-stop" data-tip="Take it off the stage for everyone" onclick={() => void stop()}>Stop sharing</button>{/if}
{/snippet}

{#if place === 'top' && mode === 'strip'}
  <div class="sp-strip" role="region" aria-label="{card.title}, on Spotify">
    {@render artwork('lg')}
    {@render words()}
    <SpotifyIcon />
    {@render openLink()}
    {@render stopBtn()}
    <button type="button" class="icon-btn icon-btn-sm" aria-label="Minimise" data-tip="Minimise" onclick={() => void show('pill')}><Icon name="min" size={16} /></button>
    <button type="button" class="icon-btn icon-btn-sm" aria-label="Hide for me" data-tip="Hide for me" onclick={() => void show('hidden')}><Icon name="x" size={16} /></button>
  </div>
  {#if error}<p class="error strip-error" role="alert">{error}</p>{/if}
{:else if place === 'corner' && mode === 'pill'}
  <!-- svelte-ignore a11y_no_static_element_interactions (the hover zone: the pill and its card are the controls; a press in a hovered card keeps it open) -->
  <div class="corner stage-corner" bind:this={zone} onpointerenter={pointerEnter} onpointermove={pointerMove} onpointerleave={pointerLeave}
    onpointerdown={() => { if (open === 'hover') open = 'pinned' }} onkeydown={keydown} onfocusout={focusOut}>
    {#if open}
      <div class="corner-card menu-surface" role="region" aria-label="{card.title}, on Spotify">
        <div class="row">
          {@render artwork('md')}
          {@render words()}
          <div class="card-ctl">
            <button type="button" class="icon-btn icon-btn-xs" aria-label="Show above the tiles" data-tip="Show above the tiles" onclick={() => void show('strip')}><Icon name="expand" size={14} /></button>
            <button type="button" class="icon-btn icon-btn-xs" aria-label="Minimise" data-tip="Minimise" onclick={() => shrink(true)}><Icon name="min" size={14} /></button>
            <button type="button" class="icon-btn icon-btn-xs" aria-label="Hide for me" data-tip="Hide for me" onclick={() => void show('hidden')}><Icon name="x" size={14} /></button>
          </div>
        </div>
        <div class="row actions">
          <SpotifyIcon />
          {@render openLink()}
          <span class="grow"></span>
          {@render stopBtn()}
        </div>
        {#if error}<p class="error" role="alert">{error}</p>{/if}
      </div>
    {:else}
      <button type="button" class="pill" aria-label="{card.title}, on Spotify: open" aria-expanded="false" onclick={() => void grow('pinned', true)}>
        {@render artwork('sm')}
        <SpotifyIcon />
        <span class="pill-title">{share.now_playing?.title ?? card.title}</span>
      </button>
    {/if}
  </div>
{/if}

<style>
  /* the strip above the tiles: the stage's 20px gutter, the QotY strip's surface */
  .sp-strip { flex-shrink: 0; margin: 12px 20px 0; display: flex; align-items: center; gap: 12px; padding: 8px; border-radius: var(--radius-md); background: var(--bg-2); border: 1px solid var(--line-soft); color: var(--text); min-width: 0; }
  .words { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .title { font-size: 14px; font-weight: 600; color: var(--text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .sub { font-size: 12px; color: var(--text-3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .sub.now { color: var(--text-2); }
  .sp-open, .sp-stop { height: 28px; padding: 0 10px; font-size: 12px; text-decoration: none; white-space: nowrap; flex-shrink: 0; }
  .art { flex: none; border-radius: 4px; overflow: hidden; display: grid; place-items: center; }
  .art img { max-width: 100%; max-height: 100%; width: auto; height: auto; object-fit: contain; display: block; border-radius: 4px; }
  .art.tile { color: var(--text-3); background: linear-gradient(135deg, var(--bg-3), var(--bg-4)); }
  .art.sm { width: 22px; height: 22px; }
  .art.md { width: 52px; height: 52px; }
  .art.lg { width: 48px; height: 48px; }
  /* the floating pill and the corner card it grows into: the stage's bottom-right corner, low over the tiles (#288),
     anchored by that corner so the card opens up and left from the pill */
  .corner { position: absolute; right: 20px; bottom: 6px; z-index: 5; display: flex; flex-direction: column; align-items: flex-end; max-width: calc(100% - 40px); }
  /* every control fits at its width: the icons beside the words, the link-out and Stop sharing below */
  .corner-card { width: 300px; max-width: 100%; box-sizing: border-box; padding: 10px; gap: 8px; color: var(--text); }
  .row { display: flex; align-items: center; gap: 8px; min-width: 0; }
  /* a narrow stage: Stop sharing drops under the link-out rather than past the card's edge */
  .actions { flex-wrap: wrap; row-gap: 6px; }
  .card-ctl { display: flex; align-self: flex-start; gap: 2px; flex-shrink: 0; }
  .grow { flex: 1; min-width: 0; }
  .pill { display: inline-flex; align-items: center; gap: 8px; max-width: 260px; height: 34px; padding: 0 12px 0 6px; border-radius: var(--radius-xl); border: 1px solid var(--line-soft); background: var(--bg-2); color: var(--text-2); font-size: 12px; box-shadow: var(--shadow-1); }
  .pill:hover { background: var(--bg-3); color: var(--text); }
  .pill-title { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .error { margin: 0; color: var(--danger-text); font-size: 12px; }
  .strip-error { margin: 6px 20px 0; }
</style>
