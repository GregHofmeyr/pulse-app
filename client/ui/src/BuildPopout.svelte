<script lang="ts">
  // The build pop-out window's page (#620, spec §4.4): one build, read-only, in a small window of its own. A bar with
  // its title and the always-on-top switch (labelled, and only where the window can be kept on top: QPMKJE), then the
  // build as BuildView reads it (title, media and notes, scrolling; never its comments). It never loads the app's
  // store: everything comes from the core, which keeps the snapshot the main window handed it. This window may call
  // only the three commands used here (ipc_gate.rs) and has no events, so it looks for a newer snapshot every couple of
  // seconds, and at once when it's brought forward (Pop out again switches it in place: #639). Screenshot links last
  // an hour: asked for again every 50 minutes. The video is its link card, opening in the browser (inline playback
  // here is #627).
  import { onMount } from 'svelte'
  import BuildView from './components/BuildView.svelte'
  import Icon from './components/Icon.svelte'
  import type { BuildScreenshot } from './lib/builds'
  import type { BuildSnapshot } from './lib/buildSnapshot'
  import { relativeTime } from './lib/conversations'
  import { isMediaHash, mediaUrl } from './lib/emoji'
  import { safeHref } from './lib/safeHref'
  import { api } from './lib/tauri'
  import { installTooltips } from './lib/tooltip'

  const LOOK_MS = 2000
  const IMAGES_MS = 50 * 60_000

  let snap = $state<BuildSnapshot | null>(null)
  let loaded = $state(false)
  /** The window opens on top (open_build_window). */
  let onTop = $state(true)
  /** The window can be kept on top here: unknown until the core says (Wayland can't), and no switch till then. */
  let canOnTop = $state(false)
  let onTopError = $state('')
  let urls = $state<Record<string, string>>({})
  let now = $state(Date.now())

  async function look() {
    try {
      const s = await api.buildWindowSnapshot()
      if (JSON.stringify(s) !== JSON.stringify(snap)) snap = s
    } catch {
      /* keep what's shown */
    }
    loaded = true
  }

  async function fetchImages() {
    const s = snap
    if (!s?.screenshots.length) return
    try {
      const list = await api.buildWindowImageUrls(s.message_id)
      if (snap?.message_id !== s.message_id) return
      urls = Object.fromEntries(list.flatMap((u) => (u.url ? [[u.id, u.url]] : [])))
    } catch {
      /* the tiles stay blank until the next try */
    }
  }

  // a different build, or its screenshots changed: links for them now
  const shotsKey = $derived(snap ? `${snap.message_id}:${snap.screenshots.map((x) => x.id).join(',')}` : '')
  $effect(() => {
    if (!shotsKey) return
    void fetchImages()
  })

  onMount(() => installTooltips())

  onMount(() => {
    // re-states the opening state; the answer says whether the switch can do anything in this window
    api.setBuildWindowOnTop(true).then((ok) => (canOnTop = ok === true), () => {})
    void look()
    const looks = setInterval(() => void look(), LOOK_MS)
    // Pop out again (this build or another) brings this window forward: it looks now, not at the next tick
    const onFocus = () => void look()
    window.addEventListener('focus', onFocus)
    const images = setInterval(() => void fetchImages(), IMAGES_MS)
    const clock = setInterval(() => (now = Date.now()), 60_000)
    return () => {
      window.removeEventListener('focus', onFocus)
      clearInterval(looks)
      clearInterval(images)
      clearInterval(clock)
    }
  })

  async function toggleOnTop() {
    const next = !onTop
    onTopError = ''
    try {
      if (await api.setBuildWindowOnTop(next)) onTop = next
      else canOnTop = false
    } catch {
      onTopError = "Couldn't change that. Try again."
    }
  }

  const screenshots = $derived<BuildScreenshot[]>(snap ? snap.screenshots.map((x) => ({ id: x.id, url: urls[x.id] ?? null, w: x.width, h: x.height })) : [])
  const link = $derived(snap?.link ?? null)
  const thumbSrc = $derived(link?.thumb && isMediaHash(link.thumb.hash) ? mediaUrl(link.thumb.hash) : null)
  const host = $derived.by(() => {
    try {
      return link ? new URL(link.url).hostname.replace(/^www\./, '') : ''
    } catch {
      return ''
    }
  })
</script>

<div class="build-popout">
  <div class="bar">
    <span class="title">{snap?.title ?? 'Build'}</span>
    {#if canOnTop}
      <button type="button" class="on-top btn btn-quiet" aria-pressed={onTop} data-tip={onTop ? 'Kept on top of other windows: click to stop' : 'Click to keep it on top of other windows'}
        onclick={() => void toggleOnTop()}><Icon name="pin" size={14} /><span>On top</span></button>
    {/if}
  </div>
  {#if onTopError}<p class="on-top-error" role="alert">{onTopError}</p>{/if}
  {#if snap}
    <div class="scroll">
      <BuildView title={snap.title} gameName={snap.game_name} guessed={snap.game_guessed} authorName={snap.author_name}
        savedAt={relativeTime(snap.saved_at, now)} updatedAt={snap.updated_at ? relativeTime(snap.updated_at, now) : null} notes={snap.notes}
        {screenshots} {link} archived={snap.archived} quickDrop={snap.quick_drop}>
        {#snippet video()}
          {#if link}
            <a class="link-card" href={safeHref(link.url)} target="_blank" rel="noopener noreferrer">
              <span class="thumb">{#if thumbSrc}<img src={thumbSrc} alt="" loading="lazy" />{:else}<Icon name={link.provider === 'generic' ? 'unlink' : 'play'} size={20} />{/if}</span>
              <span class="text">
                <span class="title">{link.title ?? link.url}</span>
                <span class="host">{host ? `${host} · ` : ''}opens in your browser</span>
              </span>
            </a>
          {/if}
        {/snippet}
      </BuildView>
    </div>
  {:else if loaded}
    <p class="gone">This build is gone.</p>
  {/if}
</div>

<style>
  :global(html), :global(body) { height: 100%; margin: 0; overflow: hidden; }
  .build-popout { height: 100vh; display: flex; flex-direction: column; min-width: 0; background: var(--bg-1); color: var(--text); }
  .bar { flex-shrink: 0; display: flex; align-items: center; gap: 6px; min-width: 0; padding: 8px 8px 8px 12px; border-bottom: 1px solid var(--bg-3); }
  .bar .title { flex: 1; min-width: 0; font-size: 13px; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .on-top { flex-shrink: 0; height: 28px; padding: 0 10px; gap: 5px; font-size: 12px; }
  .on-top[aria-pressed='true'] { color: var(--accent-text); }
  .on-top[aria-pressed='false'] { color: var(--text-3); }
  .on-top-error { flex-shrink: 0; margin: 0; padding: 6px 12px; color: var(--danger-text); font-size: 12px; overflow-wrap: anywhere; border-bottom: 1px solid var(--bg-3); }
  .scroll { flex: 1; min-height: 0; min-width: 0; overflow-y: auto; overflow-x: hidden; padding: 12px 14px 18px; }
  .gone { margin: 0; padding: 18px 14px; color: var(--text-3); font-size: 13px; }
  /* a narrow window: nothing may spill sideways (wrap or ellipsis), one screenshot under another */
  .scroll :global(.build-view h1) { font-size: 18px; }
  .scroll :global(.build-view .caption), .scroll :global(.build-view .meta) { overflow-wrap: anywhere; min-width: 0; }
  .scroll :global(.build-view .badges) { flex-wrap: wrap; }
  .scroll :global(.build-view .tiles) { grid-template-columns: minmax(0, 1fr); }
  .scroll :global(.build-view .notes pre) { overflow-x: auto; max-width: 100%; }
  .scroll :global(.build-view .notes img) { max-width: 100%; height: auto; }
  .scroll :global(.build-view .notes table) { display: block; max-width: 100%; overflow-x: auto; }
  .link-card { display: flex; align-items: center; gap: 10px; max-width: 100%; min-width: 0; box-sizing: border-box; padding: 8px; border-radius: var(--radius-md); border: 1px solid var(--bg-3); background: var(--bg-2); color: inherit; text-decoration: none; }
  .link-card:hover { border-color: var(--bg-4); }
  .link-card .thumb { flex-shrink: 0; width: 72px; aspect-ratio: 16 / 9; border-radius: var(--radius-sm); overflow: hidden; background: var(--bg-3); display: flex; align-items: center; justify-content: center; color: var(--text-3); }
  .link-card .thumb img { width: 100%; height: 100%; object-fit: cover; }
  .link-card .text { flex: 1; display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .link-card .title { font-size: 13px; font-weight: 600; color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .link-card .host { font-size: 12px; color: var(--text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
