<script lang="ts">
  // Quick add (#618, spec §4.3): paste a link or an image, or type a title, and press Enter. It saves at once as a quick
  // drop; the build last saved shows under the field, its title filling in when the link's page is fetched and a
  // guessed game marked "guessed" with a × that clears it. "Add details" opens the full form with what's here instead.
  import { onMount, tick } from 'svelte'
  import Icon from './Icon.svelte'
  import { buildGameName, buildShots } from '../lib/builds'
  import { classifyPaste, classifyText, type BuildPaste, type QuickPrefill } from '../lib/buildPaste'
  import { app } from '../lib/store.svelte'
  import { api, errorText, type StagedFile } from '../lib/tauri'
  import type { Message } from '../lib/protocol/Message'

  let { channelId, serverId, initial = null, onDetails, onClose }: {
    channelId: string
    serverId: string
    /** A paste that opened it (in the browse view). */
    initial?: BuildPaste | null
    onDetails: (p: QuickPrefill) => void
    onClose: () => void
  } = $props()

  let input: HTMLInputElement | undefined = $state()
  let value = $state('')
  let shot = $state<StagedFile | null>(null)
  let staging = $state(false)
  let error = $state('')
  /** The build last saved here: in flight (no id yet), then its id. */
  let last = $state<{ nonce: string; label: string; message: Message | null } | null>(null)

  /** A paste from elsewhere in the browse view, or one into the field. */
  export async function take(p: BuildPaste) {
    error = ''
    if (p.kind === 'image') await stage(p.file)
    else value = p.kind === 'link' ? p.url : p.text
    focus()
  }
  export function focus() {
    input?.focus()
  }

  async function stage(f: File) {
    staging = true
    try {
      const old = shot
      shot = await api.stageAttachmentBytes(f.name || 'pasted', new Uint8Array(await f.arrayBuffer()))
      if (old) void api.unstageAttachment(old.id).catch(() => {})
    } catch (e) {
      error = errorText(e)
    } finally {
      staging = false
    }
  }

  function onPaste(e: ClipboardEvent) {
    const p = e.clipboardData ? classifyPaste(e.clipboardData) : null
    // text goes into the field as typed; an image is staged instead
    if (p?.kind !== 'image') return
    e.preventDefault()
    e.stopPropagation()
    void stage(p.file)
  }

  /** The save that failed and was put back: tried again unchanged, it's the same save (its nonce, so its uploads are
   *  reused); changed or given up, its uploads are let go. */
  let failed: { nonce: string; value: string; shotId: string | null } | null = null
  function giveUp() {
    if (failed) void api.cancelBuildSave(failed.nonce).catch(() => {})
    failed = null
  }
  /** Closed: a failed save is given up, and a screenshot left in the field unstaged. */
  function close() {
    giveUp()
    if (shot) void api.unstageAttachment(shot.id).catch(() => {})
    shot = null
    onClose()
  }

  async function save() {
    const typed = classifyText(value)
    if (!typed && !shot) return
    const same = !!failed && failed.value === value && failed.shotId === (shot?.id ?? null)
    if (!same) giveUp()
    const nonce = same ? failed!.nonce : crypto.randomUUID()
    failed = null
    const files = shot ? [shot.id] : []
    const label = typed ? (typed.kind === 'link' ? typed.url : typed.text) : (shot?.name ?? '')
    last = { nonce, label, message: null }
    error = ''
    const kept = { value, shot }
    value = ''
    shot = null
    try {
      const m = await api.createBuild(channelId, {
        title: typed?.kind === 'text' ? typed.text : null, game_id: null, link_url: typed?.kind === 'link' ? typed.url : null, notes: null,
        attachments: [], nonce,
      }, files)
      if (last?.nonce === nonce) last = { nonce, label, message: m }
    } catch (e) {
      error = errorText(e)
      if (last?.nonce === nonce) last = null
      // put it back to try again (the same files, already uploaded, go with it)
      if (!value && !shot) {
        ;({ value, shot } = kept)
        failed = { nonce, value: kept.value, shotId: kept.shot?.id ?? null }
      } else void api.cancelBuildSave(nonce).catch(() => {}) // something new was typed meanwhile: this save is over
    }
    await tick()
    focus()
  }

  function details() {
    giveUp() // the form saves with its own nonce
    const typed = classifyText(value)
    onDetails({ title: typed?.kind === 'text' ? typed.text : '', link: typed?.kind === 'link' ? typed.url : '', files: shot ? [shot] : [] })
  }

  function key(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.isComposing) {
      e.preventDefault()
      void save()
    } else if (e.key === 'Escape') {
      e.preventDefault()
      e.stopPropagation()
      close()
    }
  }

  // The last saved build as it is now (a feed's copy follows the unfurl's update), else as it was saved.
  const saved = $derived.by(() => {
    const m = last?.message
    if (!m) return null
    for (const f of Object.values(app.state.buildFeeds)) {
      if (f.channelId !== channelId) continue
      const live = f.items.find((x) => x.id === m.id)
      if (live) return live
    }
    return m
  })
  const game = $derived(saved ? buildGameName(app.state, serverId, saved) : null)
  let clearing = $state(false)
  async function clearGame() {
    const m = saved
    if (!m?.build || clearing) return
    clearing = true
    try {
      await api.updateBuild(m.id, channelId, {
        title: m.build.title, game_id: null, link_url: m.build.link?.url ?? null, notes: m.build.notes, attachments: [], mark_sorted: false,
      }, buildShots(m).map((a) => `id:${a.id}`))
    } catch (e) {
      error = errorText(e)
    } finally {
      clearing = false
    }
  }

  onMount(() => {
    if (initial) void take(initial)
    else focus()
  })
</script>

<div class="quick-add">
  <div class="row">
    <Icon name="plus" size={16} />
    {#if shot}
      <span class="shot">{#if shot.preview}<img src={shot.preview} alt="" />{/if}{shot.name}
        <button type="button" class="icon-btn icon-btn-sm" aria-label="Remove screenshot" onclick={() => { void api.unstageAttachment(shot!.id).catch(() => {}); shot = null }}><Icon name="x" size={12} /></button>
      </span>
    {/if}
    <input bind:this={input} class="input" type="text" aria-label="Quick add a build" placeholder="Paste a link or an image, or type a title, then press Enter"
      maxlength="2048" bind:value onpaste={onPaste} onkeydown={key} />
    <button type="button" class="btn btn-primary" disabled={staging || (!value.trim() && !shot)} onclick={() => void save()}>Save</button>
    <button type="button" class="btn btn-quiet details" onclick={details}>Add details</button>
    <button type="button" class="icon-btn icon-btn-sm" aria-label="Close quick add" onclick={close}><Icon name="x" size={14} /></button>
  </div>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  <!-- one live region, always there, so a screen reader hears "Saving…" and then "Saved" -->
  <div class="status" aria-live="polite">
    {#if last && !saved}
      <p class="saved pending"><span class="title">{last.label}</span><span class="state">Saving…</span></p>
    {:else if saved?.build}
      <p class="saved">
        {#if game}
          <span class="caption">{game}{#if saved.build.game_guessed}<span class="guessed"> · guessed</span>
            <button type="button" class="clear-game" aria-label="Not {game}: clear the game" data-tip="Not this game" disabled={clearing} onclick={() => void clearGame()}><Icon name="x" size={11} /></button>{/if}</span>
        {/if}
        <span class="title">{saved.build.title}</span>
        <span class="state">Saved</span>
      </p>
    {/if}
  </div>
</div>

<style>
  .quick-add { flex-shrink: 0; padding: 10px 18px; border-bottom: 1px solid var(--bg-3); display: flex; flex-direction: column; gap: 6px; }
  .row { display: flex; align-items: center; gap: 8px; color: var(--text-3); }
  .row input { flex: 1; min-width: 0; }
  .shot { display: inline-flex; align-items: center; gap: 6px; max-width: 200px; padding: 2px 4px 2px 6px; border-radius: var(--radius-sm); background: var(--bg-3); color: var(--text-2); font-size: 12px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .shot img { width: 24px; height: 18px; object-fit: cover; border-radius: 2px; }
  .error { margin: 0; color: var(--danger-text); font-size: 13px; }
  /* nothing to say yet: no gap for it */
  .status:empty { margin-top: -6px; }
  .saved { margin: 0; display: flex; align-items: center; gap: 8px; min-width: 0; font-size: 13px; color: var(--text-2); }
  .saved .caption { display: inline-flex; align-items: center; gap: 2px; font-size: 11px; font-weight: 700; letter-spacing: .06em; text-transform: uppercase; color: var(--text-3); }
  .saved .guessed { text-transform: none; letter-spacing: 0; font-weight: 400; font-style: italic; }
  .clear-game { display: inline-flex; padding: 2px; border: 0; border-radius: var(--radius-xs); background: transparent; color: var(--text-3); cursor: pointer; }
  .clear-game:hover { background: var(--bg-3); color: var(--text); }
  .saved .title { font-weight: 600; color: var(--text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; min-width: 0; }
  .saved .state { color: var(--text-3); flex-shrink: 0; }
</style>
