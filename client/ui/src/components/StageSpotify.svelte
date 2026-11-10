<script lang="ts">
  // A Spotify link shared to the voice stage (#234), as you chose to see it (lib/stageShareView): the strip above
  // the tiles, or a pill in the stage's bottom-right corner (hover previews the corner card, a click opens it), or
  // the corner card itself. Hidden, it isn't here at all: the header's Spotify button brings it back.
  //
  // Spotify's rules as on the #196 card: their single-colour icon, never over the artwork; the artwork as they give
  // it (uncropped, 4px corners, nothing on top); their approved link-out wording; Pulse never controls anyone's Spotify.
  import { tick } from 'svelte'
  import Icon from './Icon.svelte'
  import SpotifyIcon from './SpotifyIcon.svelte'
  import { isMediaHash, mediaUrl } from '../lib/emoji'
  import { closeTimer } from '../lib/hoverIntent'
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
  const art = $derived(share.embed.thumb && isMediaHash(share.embed.thumb.hash) ? mediaUrl(share.embed.thumb.hash) : null)

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
    corner: '.stage-corner .corner-card button',
    hidden: '[data-stage-spotify]',
  }
  async function show(m: StageMode) {
    const had = !!(document.activeElement && document.activeElement !== document.body)
    setMode(share.id, m)
    await tick()
    if (had && (!document.activeElement || document.activeElement === document.body)) document.querySelector<HTMLElement>(FOCUS[m])?.focus()
  }

  // Hovering the pill previews the corner card; the pointer may move onto the card without it closing.
  let preview = $state(false)
  const leave = closeTimer()
  const enter = () => {
    leave.cancel()
    preview = true
  }
  const exit = () => leave.schedule(() => (preview = false))
  $effect(() => () => leave.cancel())
  $effect(() => {
    if (mode !== 'pill') preview = false
  })
</script>

{#snippet artwork(size: 'sm' | 'md' | 'lg')}
  <!-- Spotify's artwork as they give it: uncropped, 4px corners, nothing over it -->
  <span class="art {size}" class:tile={!art} aria-hidden="true">
    {#if art}<img src={art} alt="" width={share.embed.thumb!.width} height={share.embed.thumb!.height} />{:else}<Icon name="headphones" size={size === 'sm' ? 14 : 18} />{/if}
  </span>
{/snippet}

{#snippet words()}
  <span class="words">
    <span class="title" data-tip={card.title}>{card.title}</span>
    <span class="sub" data-tip={card.subtitle}>{card.subtitle}</span>
  </span>
{/snippet}

{#snippet open()}
  <a class="btn {card.jam ? 'btn-primary' : 'btn-secondary'} sp-open" href={share.embed.url} target="_blank" rel="noopener noreferrer">{card.action}</a>
{/snippet}

{#snippet stopBtn()}
  {#if canStop}<button type="button" class="btn btn-quiet sp-stop" data-tip="Take it off the stage for everyone" onclick={() => void stop()}>Stop sharing</button>{/if}
{/snippet}

{#snippet cornerCard(previewing: boolean)}
  <div class="corner-card menu-surface" class:pop-in={!previewing} role={previewing ? undefined : 'region'} aria-label={previewing ? undefined : `${card.title}, on Spotify`}
    onpointerenter={previewing ? enter : undefined} onpointerleave={previewing ? exit : undefined}>
    <div class="row">
      {@render artwork('md')}
      {@render words()}
    </div>
    <div class="row">
      <SpotifyIcon />
      {@render open()}
      <span class="grow"></span>
      {#if !previewing}
        {@render stopBtn()}
        <button type="button" class="icon-btn icon-btn-xs" aria-label="Show above the tiles" data-tip="Show above the tiles" onclick={() => void show('strip')}><Icon name="square" size={14} /></button>
        <button type="button" class="icon-btn icon-btn-xs" aria-label="Minimise" data-tip="Minimise" onclick={() => void show('pill')}><Icon name="min" size={14} /></button>
        <button type="button" class="icon-btn icon-btn-xs" aria-label="Hide for me" data-tip="Hide for me" onclick={() => void show('hidden')}><Icon name="x" size={14} /></button>
      {/if}
    </div>
    {#if error && !previewing}<p class="error" role="alert">{error}</p>{/if}
  </div>
{/snippet}

{#if place === 'top' && mode === 'strip'}
  <div class="sp-strip" role="region" aria-label="{card.title}, on Spotify">
    {@render artwork('lg')}
    {@render words()}
    <SpotifyIcon />
    {@render open()}
    {@render stopBtn()}
    <button type="button" class="icon-btn icon-btn-sm" aria-label="Minimise" data-tip="Minimise" onclick={() => void show('pill')}><Icon name="min" size={16} /></button>
    <button type="button" class="icon-btn icon-btn-sm" aria-label="Hide for me" data-tip="Hide for me" onclick={() => void show('hidden')}><Icon name="x" size={16} /></button>
  </div>
  {#if error}<p class="error strip-error" role="alert">{error}</p>{/if}
{:else if place === 'corner' && mode === 'corner'}
  <div class="corner stage-corner">{@render cornerCard(false)}</div>
{:else if place === 'corner' && mode === 'pill'}
  <div class="corner stage-corner">
    {#if preview}{@render cornerCard(true)}{/if}
    <button type="button" class="pill" aria-label="{card.title}, on Spotify: open" onclick={() => void show('corner')}
      onpointerenter={enter} onpointerleave={exit} onfocus={enter} onblur={exit}>
      {@render artwork('sm')}
      <SpotifyIcon />
      <span class="pill-title">{card.title}</span>
    </button>
  </div>
{/if}

<style>
  /* the strip above the tiles: the stage's 20px gutter, the QotY strip's surface */
  .sp-strip { flex-shrink: 0; margin: 12px 20px 0; display: flex; align-items: center; gap: 12px; padding: 8px; border-radius: var(--radius-md); background: var(--bg-2); border: 1px solid var(--line-soft); color: var(--text); min-width: 0; }
  .words { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .title { font-size: 14px; font-weight: 600; color: var(--text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .sub { font-size: 12px; color: var(--text-3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .sp-open, .sp-stop { height: 28px; padding: 0 10px; font-size: 12px; text-decoration: none; white-space: nowrap; flex-shrink: 0; }
  .art { flex: none; border-radius: 4px; overflow: hidden; display: grid; place-items: center; }
  .art img { max-width: 100%; max-height: 100%; width: auto; height: auto; object-fit: contain; display: block; border-radius: 4px; }
  .art.tile { color: var(--text-3); background: linear-gradient(135deg, var(--bg-3), var(--bg-4)); }
  .art.sm { width: 22px; height: 22px; }
  .art.md { width: 52px; height: 52px; }
  .art.lg { width: 48px; height: 48px; }
  /* the floating pill and the corner card: the stage's bottom-right corner, over the tiles */
  .corner { position: absolute; right: 20px; bottom: 12px; z-index: 5; display: flex; flex-direction: column; align-items: flex-end; gap: 8px; max-width: calc(100% - 40px); }
  .corner-card { width: 280px; max-width: 100%; box-sizing: border-box; padding: 10px; gap: 8px; color: var(--text); }
  .row { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .grow { flex: 1; }
  .pill { display: inline-flex; align-items: center; gap: 8px; max-width: 260px; height: 34px; padding: 0 12px 0 6px; border-radius: var(--radius-xl); border: 1px solid var(--line-soft); background: var(--bg-2); color: var(--text-2); font-size: 12px; box-shadow: var(--shadow-1); }
  .pill:hover { background: var(--bg-3); color: var(--text); }
  .pill-title { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .error { margin: 0; color: var(--danger-text); font-size: 12px; }
  .strip-error { margin: 6px 20px 0; }
</style>
