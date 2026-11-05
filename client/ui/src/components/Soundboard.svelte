<script lang="ts">
  // The soundboard popover's content: search, a rail, sections of 3-column tiles. Modelled on the
  // emoji picker. Task 9's add/edit dialog and tidy-up list open through `onAdd`, `onEdit`, `onTidy`.
  import { tick } from 'svelte'
  import type { Server } from '../lib/protocol/Server'
  import type { Sound } from '../lib/protocol/Sound'
  import { app } from '../lib/store.svelte'
  import { mediaUrl } from '../lib/emoji'
  import { isNotFound, isRateLimited, previewPercents, searchSounds, soundSections, TIDY_THRESHOLD, type Section } from '../lib/soundboard'
  import { setSoundboardVolume, soundboard } from '../lib/soundboard.svelte'
  import { mixer } from '../lib/mixer.svelte'
  import { errorText, soundboardCmd } from '../lib/tauri'
  import { loadAudioConfig } from '../lib/voiceui'
  import Icon from './Icon.svelte'
  import SoundEmoji from './SoundEmoji.svelte'

  let { onClose, onAdd, onEdit, onTidy, servers, memberOf }: {
    onClose: () => void
    /** Task 9: open the add-sound dialog for this server. */
    onAdd: (serverId: string) => void
    /** Task 9: open the edit dialog for this sound. */
    onEdit: (sound: Sound) => void
    /** Task 9: open the tidy-up list for this server. */
    onTidy: (serverId: string) => void
    servers: Server[]
    memberOf: Set<string>
  } = $props()

  let query = $state('')
  let showVolume = $state(false)
  let error = $state<string | null>(null)
  let menu = $state<{ sound: Sound; x: number; y: number } | null>(null)
  let asking = $state<Sound | null>(null)
  let root: HTMLDivElement
  let searchEl: HTMLInputElement

  const sections = $derived(soundSections(app.state, memberOf, servers))
  const everything = $derived(Object.values(app.state.sounds).filter((s) => memberOf.has(s.server_id)))
  const results = $derived(query.trim() ? searchSounds(everything, query) : null)
  const favourites = $derived(new Set(app.state.soundFavourites))
  const countIn = (serverId: string) => everything.filter((s) => s.server_id === serverId).length

  $effect(() => {
    searchEl?.focus()
  })

  async function play(s: Sound) {
    error = null
    try {
      await soundboardCmd.playToCall(s.id)
    } catch (e) {
      // a stale click (the sound was just deleted) or the abuse limit: nothing worth saying
      if (!isRateLimited(e) && !isNotFound(e)) error = errorText(e)
    }
  }
  // Local only: never touches the server, so nobody else hears it.
  function preview(s: Sound) {
    error = null
    const [calls, level] = previewPercents(mixer.calls.master, soundboard.volume_pct)
    soundboardCmd.play(s.media_hash, calls, level, s.volume, loadAudioConfig().output ?? null).catch((e) => (error = errorText(e)))
  }
  function favourite(s: Sound) {
    soundboardCmd.setFavourite(s.id, !favourites.has(s.id)).catch((e) => (error = errorText(e)))
  }
  async function remove(s: Sound) {
    asking = null
    try {
      await soundboardCmd.remove(s.id)
    } catch (e) {
      if (!isNotFound(e)) error = errorText(e) // someone else deleted it first: same result
    }
  }

  function jump(id: string) {
    query = '' // sections are hidden while a search is showing
    void tick().then(() => root.querySelector(`[data-section="${id}"]`)?.scrollIntoView({ block: 'start' }))
  }

  function openMenu(ev: MouseEvent, s: Sound) {
    ev.preventDefault()
    const r = root.getBoundingClientRect()
    // inside the popover so its outside-click handling covers it; clamped to the popover's edges
    menu = { sound: s, x: Math.max(4, Math.min(ev.clientX - r.left, r.width - 164)), y: Math.max(4, Math.min(ev.clientY - r.top, r.height - 120)) }
  }
  const choose = (action: (s: Sound) => void) => {
    const s = menu?.sound
    menu = null
    if (s) action(s)
  }

  function onKey(ev: KeyboardEvent) {
    if (ev.key !== 'Escape') return
    ev.preventDefault()
    // Escape peels back the innermost thing first
    if (asking) asking = null
    else if (menu) menu = null
    else onClose()
  }
  function tileKey(ev: KeyboardEvent, s: Sound) {
    if (ev.target !== ev.currentTarget) return
    if (ev.key === 'Enter' || ev.key === ' ') {
      ev.preventDefault()
      void play(s)
    }
  }
  const initial = (s: Server) => s.name.trim().charAt(0).toUpperCase()
</script>

{#snippet tileFor(s: Sound)}
  <div
    class="tile" role="button" tabindex="0" data-tip={s.name}
    onclick={() => play(s)} onkeydown={(ev) => tileKey(ev, s)} oncontextmenu={(ev) => openMenu(ev, s)}
  >
    <button class="preview" type="button" aria-label="Preview {s.name}" onclick={(ev) => { ev.stopPropagation(); preview(s) }}><Icon name="volume" size={14} /></button>
    <span class="emoji"><SoundEmoji emoji={s.emoji} emojiId={s.emoji_id} size={24} /></span>
    <span class="name">{s.name}</span>
  </div>
{/snippet}

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div class="board" bind:this={root} role="dialog" aria-label="Soundboard" tabindex="-1" onkeydown={onKey}>
  <div class="top">
    <input bind:this={searchEl} bind:value={query} class="search input input-sm" type="text" placeholder="Find a sound" aria-label="Find a sound" autocomplete="off" spellcheck="false" />
    <button class="vol-btn" class:on={showVolume} type="button" aria-label="Soundboard volume" aria-expanded={showVolume} onclick={() => (showVolume = !showVolume)}>
      <Icon name={soundboard.volume_pct === 0 ? 'speakerOff' : 'volume'} size={18} />
    </button>
  </div>
  {#if showVolume}
    <div class="vol-row">
      <span>Soundboard volume</span>
      <input type="range" min="0" max="200" step="1" value={soundboard.volume_pct} aria-label="Soundboard volume level" aria-valuetext="{soundboard.volume_pct}%"
        oninput={(e) => setSoundboardVolume(+e.currentTarget.value)} />
      <strong>{soundboard.volume_pct}%</strong>
    </div>
  {/if}
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  <div class="body">
    <nav class="rail" aria-label="Soundboard sections">
      {#each sections as s (s.id)}
        <button class="rail-btn" type="button" aria-label={s.label} data-rail={s.id} onclick={() => jump(s.id)}>
          {#if s.id === 'favourites'}
            <Icon name="star" size={18} />
          {:else if s.id === 'frequent'}
            <Icon name="clock" size={18} />
          {:else if s.server?.icon_hash}
            <img class="srv" src={mediaUrl(s.server.icon_hash)} alt="" />
          {:else if s.server}
            <span class="srv init">{initial(s.server)}</span>
          {/if}
        </button>
      {/each}
    </nav>
    <div class="grid">
      {#if results}
        {#if results.length === 0}
          <p class="none">No sounds found</p>
        {:else}
          <div class="tiles" data-cols="3">
            {#each results as s (s.id)}{@render tileFor(s)}{/each}
          </div>
        {/if}
      {:else}
        {#each sections as sec (sec.id)}
          <section data-section={sec.id}>
            <div class="head">
              <span class="label">{sec.label}</span>
              {#if sec.server && countIn(sec.server.id) > TIDY_THRESHOLD}
                <button class="link-btn tidy" type="button" onclick={() => onTidy(sec.server!.id)}>Tidy up?</button>
              {/if}
            </div>
            <div class="tiles" data-cols="3">
              {#each sec.sounds as s (s.id)}{@render tileFor(s)}{/each}
              {#if sec.server}
                <button class="add-tile" type="button" onclick={() => onAdd(sec.server!.id)}><Icon name="plus" size={20} /><span>Add sound</span></button>
              {/if}
            </div>
          </section>
        {/each}
      {/if}
    </div>
  </div>

  {#if menu}
    {@const m = menu}
    <div class="ctx menu-surface pop-in" role="menu" aria-label="Actions for {m.sound.name}" style:left="{m.x}px" style:top="{m.y}px">
      <button type="button" class="menu-item" role="menuitem" onclick={() => choose(favourite)}>{favourites.has(m.sound.id) ? 'Unfavourite' : 'Favourite'}</button>
      <button type="button" class="menu-item" role="menuitem" onclick={() => choose(onEdit)}>Edit</button>
      <button type="button" class="menu-item danger" role="menuitem" onclick={() => choose((s) => (asking = s))}>Delete</button>
    </div>
  {/if}
  {#if asking}
    {@const a = asking}
    <div class="scrim">
      <div class="ask menu-surface pop-in" role="alertdialog" aria-label="Delete {a.name}">
        <p>Delete {a.name}?</p>
        <div class="btns">
          <button type="button" class="btn btn-secondary" onclick={() => (asking = null)}>Cancel</button>
          <button type="button" class="btn btn-danger" onclick={() => remove(a)}>Delete</button>
        </div>
      </div>
    </div>
  {/if}
</div>

<style>
  /* the menus' frame (#173: .menu-surface's border, radius and shadow) round a bg-2 panel: its tiles, rail and footer
     are drawn for bg-2, so the fill stays */
  .board { position: relative; width: 440px; height: 460px; display: flex; flex-direction: column; background: var(--bg-2); border: 1px solid var(--line-menu); border-radius: var(--radius-lg); box-shadow: var(--shadow-3); overflow: hidden; color: var(--text); outline: none; }
  .top { display: flex; gap: 8px; align-items: center; padding: 10px; border-bottom: 1px solid var(--line); }
  /* the shared small field (.input .input-sm) */
  .search { flex: 1; min-width: 0; }
  .vol-btn { width: 32px; height: 32px; display: grid; place-items: center; border: 0; border-radius: 8px; background: transparent; color: var(--text-2); padding: 0; cursor: pointer; }
  .vol-btn:hover, .vol-btn.on { background: var(--bg-3); color: var(--text); }
  .vol-row { display: flex; align-items: center; gap: 10px; padding: 8px 12px; background: var(--bg-1); border-bottom: 1px solid var(--line); font-size: 12px; color: var(--text-2); }
  .vol-row input { flex: 1; accent-color: var(--accent); }
  .vol-row strong { width: 38px; text-align: right; color: var(--text); font-weight: 600; }
  .error { margin: 0; padding: 6px 12px; color: var(--danger-text); font-size: 12px; border-bottom: 1px solid var(--line); }
  .body { flex: 1; min-height: 0; display: flex; }
  .rail { display: flex; flex-direction: column; gap: 4px; padding: 6px; background: var(--bg-1); border-right: 1px solid var(--line); overflow-y: auto; }
  .rail-btn { width: 30px; height: 30px; flex-shrink: 0; display: grid; place-items: center; border: 0; border-radius: 8px; background: transparent; color: var(--text-2); cursor: pointer; padding: 0; }
  .rail-btn:hover { background: var(--bg-3); color: var(--text); }
  .srv { width: 24px; height: 24px; border-radius: 50%; object-fit: cover; }
  .init { display: grid; place-items: center; background: var(--bg-4); color: var(--text); font-size: 12px; font-weight: 700; }
  .grid { flex: 1; min-width: 0; overflow-y: auto; padding: 0 8px 8px; }
  .head { position: sticky; top: 0; z-index: 1; display: flex; align-items: center; justify-content: space-between; gap: 8px; padding: 8px 2px 4px; background: var(--bg-2); color: var(--text-3); font-size: 11px; font-weight: 700; text-transform: uppercase; letter-spacing: 0.04em; }
  .tidy { font-size: 11px; text-transform: none; letter-spacing: 0; font-weight: 600; }
  .tiles { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 6px; }
  .tile { position: relative; min-width: 0; height: 72px; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 4px; padding: 6px 4px; border-radius: 10px; background: var(--bg-3); cursor: pointer; }
  .tile:hover { background: var(--bg-4); }
  .emoji { font-size: 24px; line-height: 1; height: 26px; display: grid; place-items: center; color: var(--text-2); }
  .name { max-width: 100%; font-size: 12px; font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .preview { position: absolute; top: 3px; right: 3px; width: 20px; height: 20px; display: grid; place-items: center; padding: 0; border: 0; border-radius: 6px; background: transparent; color: var(--text-3); cursor: pointer; opacity: 0; }
  .tile:hover .preview, .tile:focus-within .preview { opacity: 1; }
  .preview:hover { background: var(--bg-2); color: var(--text); }
  .add-tile { height: 72px; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 4px; border: 1px dashed var(--line); border-radius: 10px; background: transparent; color: var(--text-2); font: inherit; font-size: 12px; cursor: pointer; }
  .add-tile:hover { background: var(--bg-3); color: var(--text); }
  .none { color: var(--text-3); font-size: 13px; text-align: center; margin: 24px 0; }
  .ctx { position: absolute; z-index: 5; min-width: 150px; gap: 2px; font-size: 13px; --pop-origin: top left; }
  .scrim { position: absolute; inset: 0; z-index: 6; display: grid; place-items: center; background: var(--scrim); }
  .ask { width: 240px; padding: 12px; gap: 10px; }
  .ask p { margin: 0; font-size: 13px; color: var(--text); }
  .btns { display: flex; justify-content: flex-end; gap: 6px; }
</style>
