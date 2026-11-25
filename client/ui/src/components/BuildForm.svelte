<script lang="ts">
  // A build's full form (#618, spec §4.3), for a new build ("Add details") and for Edit: Title (required), Game (the
  // registry, or "+ Use 'X' as a new game"), Build link, Screenshots (paste, drop or pick; drag or the arrows to
  // reorder; × to remove; at most 10) and Notes. An image pasted anywhere on the form, even while typing in Notes, goes
  // to Screenshots; pasted text stays text. "Mark as sorted" ends a quick drop that has no game or notes.
  import { tick, untrack } from 'svelte'
  import GamePicker from './GamePicker.svelte'
  import Icon from './Icon.svelte'
  import { attachmentLink } from '../lib/attachmentLinks'
  import { modal } from '../lib/actions'
  import { buildShots } from '../lib/builds'
  import { TITLE_MAX, type QuickPrefill } from '../lib/buildPaste'
  import { api, errorText, type StagedFile } from '../lib/tauri'
  import type { Attachment } from '../lib/protocol/Attachment'
  import type { BuildGameEdit } from '../lib/protocol/BuildGameEdit'
  import type { Message } from '../lib/protocol/Message'

  let { channelId, serverId, build = null, prefill = null, onClose, onSaved = () => {} }: {
    channelId: string
    serverId: string
    /** The build being edited; none for a new one. */
    build?: Message | null
    /** A new build's start (quick add's "Add details"). */
    prefill?: QuickPrefill | null
    onClose: () => void
    onSaved?: (m: Message) => void
  } = $props()

  const MAX_SHOTS = 10
  const MAX_NOTES = 4000
  const MAX_LINK = 2048
  const TOO_MANY = 'Up to 10 screenshots.'

  type Shot = { key: string; kind: 'kept'; attachment: Attachment; url: string | null } | { key: string; kind: 'staged'; file: StagedFile }

  // One form per build (or new one): what it opened with.
  const editing = untrack(() => build)
  const start = untrack(() => prefill)
  const b = editing?.build ?? null
  let title = $state(b?.title ?? start?.title ?? '')
  // A guessed game (from the link's title) shows in the box, marked as guessed, until the person keeps it, picks
  // another or clears it; saved untouched, it's sent as "unchanged" and stays a guess (and a quick drop stays one).
  let game = $state<string | null>(b?.game_id ?? null)
  let guessed = $state(!!b?.game_guessed && !!b?.game_id)
  let link = $state(b?.link?.url ?? start?.link ?? '')
  let notes = $state(b?.notes ?? '')
  let sorted = $state(false)
  let shots = $state<Shot[]>([
    ...(editing ? buildShots(editing).map((a): Shot => ({ key: `id:${a.id}`, kind: 'kept', attachment: a, url: null })) : []),
    ...(start?.files ?? []).map((f): Shot => ({ key: `staged:${f.id}`, kind: 'staged', file: f })),
  ])
  let shotsNote = $state('')
  let busy = $state(false)
  let error = $state('')
  // one nonce per form: a retried save reuses the screenshots it already uploaded
  const nonce = crypto.randomUUID()
  let failedSave = false

  const titleLen = $derived([...title.trim()].length)
  const canSave = $derived(titleLen > 0 && titleLen <= TITLE_MAX && !busy)
  const offerSorted = $derived(!!b?.quick_drop && (!game || guessed) && !notes.trim())

  // kept screenshots' links, for their thumbnails
  $effect(() => {
    if (!editing) return
    for (const s of shots) {
      if (s.kind !== 'kept' || s.url) continue
      const key = s.key
      attachmentLink(editing.id, s.attachment.id, api.attachmentUrls).then(
        (u) => { const x = shots.find((y) => y.key === key); if (x?.kind === 'kept') x.url = u },
        () => {},
      )
    }
  })

  const shotName = (s: Shot) => (s.kind === 'kept' ? s.attachment.name : s.file.name)
  const shotSrc = (s: Shot) => (s.kind === 'kept' ? s.url : s.file.preview)

  function room(): boolean {
    if (shots.length < MAX_SHOTS) return true
    shotsNote = TOO_MANY
    return false
  }
  function addStaged(f: StagedFile) {
    if (f.kind !== 'image') {
      void api.unstageAttachment(f.id).catch(() => {})
      shotsNote = 'Screenshots are images.'
      return
    }
    if (!room()) return void api.unstageAttachment(f.id).catch(() => {})
    shots.push({ key: `staged:${f.id}`, kind: 'staged', file: f })
  }
  async function addFiles(files: File[]) {
    shotsNote = ''
    for (const f of files) {
      if (!f.type.startsWith('image/')) {
        shotsNote = 'Screenshots are images.'
        continue
      }
      if (!room()) return
      try {
        addStaged(await api.stageAttachmentBytes(f.name || 'pasted', new Uint8Array(await f.arrayBuffer())))
      } catch (e) {
        shotsNote = errorText(e)
      }
    }
  }
  async function pick() {
    shotsNote = ''
    const paths = await api.pickAttachmentFiles().catch(() => [] as string[])
    if (!paths.length) return
    try {
      for (const r of await api.stageAttachmentPaths(paths)) {
        if (r.status === 'staged') addStaged(r.file)
        else shotsNote = r.error
      }
    } catch (e) {
      shotsNote = errorText(e)
    }
  }

  // An image pasted anywhere on the form goes to Screenshots (before the field it was pasted in sees it); text is left
  // to the field.
  function pasteCapture(e: ClipboardEvent) {
    const images = Array.from(e.clipboardData?.files ?? []).filter((f) => f.type.startsWith('image/'))
    if (!images.length) return
    e.preventDefault()
    e.stopPropagation()
    void addFiles(images)
  }
  function drop(e: DragEvent) {
    const files = Array.from(e.dataTransfer?.files ?? [])
    if (!files.length) return
    e.preventDefault()
    void addFiles(files)
  }

  function move(i: number, by: number) {
    const j = i + by
    if (j < 0 || j >= shots.length) return
    const next = [...shots]
    ;[next[i], next[j]] = [next[j], next[i]]
    shots = next
  }
  function remove(i: number) {
    const [s] = shots.splice(i, 1)
    if (s?.kind === 'staged') void api.unstageAttachment(s.file.id).catch(() => {})
    shotsNote = ''
  }
  // drag to reorder
  let dragging = $state<number | null>(null)
  function dragOver(e: DragEvent, i: number) {
    if (dragging === null) return
    e.preventDefault()
    if (dragging !== i) {
      const next = [...shots]
      const [s] = next.splice(dragging, 1)
      next.splice(i, 0, s)
      shots = next
      dragging = i
    }
  }

  // an edit's game: an untouched guess is left as it is; anything else is the person's pick (or none)
  const gameEdit = (): BuildGameEdit => (guessed ? 'unchanged' : game ? { set: game } : 'clear')

  async function save() {
    if (!canSave) return
    busy = true
    error = ''
    const fields = { title: title.trim(), game_id: game, link_url: link.trim() || null, notes: notes.trim() || null, attachments: [] as string[] }
    try {
      const m = editing
        ? await api.updateBuild(editing.id, channelId, { ...fields, game: gameEdit(), mark_sorted: offerSorted && sorted }, shots.map((s) => s.key))
        : await api.createBuild(channelId, { ...fields, nonce }, shots.map((s) => (s.kind === 'staged' ? s.file.id : '')).filter(Boolean))
      onSaved(m)
      onClose()
    } catch (e) {
      error = errorText(e)
      failedSave = true
    } finally {
      busy = false
    }
  }
  function cancel() {
    if (busy) return
    // a failed save's uploads are let go (an edit's go by the build's id, as the core keeps them); a new build's
    // screenshots go back, and an edit's new ones too
    if (failedSave) void api.cancelBuildSave(editing ? `build-edit:${editing.id}` : nonce).catch(() => {})
    for (const s of shots) if (s.kind === 'staged') void api.unstageAttachment(s.file.id).catch(() => {})
    onClose()
  }

  let titleEl: HTMLInputElement | undefined = $state()
  $effect(() => {
    void tick().then(() => titleEl?.focus())
  })
</script>

<div class="overlay modal-overlay fade-in">
  <div class="backdrop modal-backdrop" role="presentation" data-backdrop onclick={cancel}></div>
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div class="modal build-form pop-in" role="dialog" aria-modal="true" aria-label={editing ? 'Edit build' : 'Add build'} tabindex="-1" use:modal={cancel}
    onpastecapture={pasteCapture} ondragover={(e) => { if (e.dataTransfer?.types.includes('Files')) e.preventDefault() }} ondrop={drop}>
    <h1 class="modal-title">{editing ? 'Edit build' : 'Add build'}</h1>
    <label class="field">
      <span class="label">Title <span class="counter" class:over={titleLen > TITLE_MAX}>{titleLen}/{TITLE_MAX}</span></span>
      <input bind:this={titleEl} class="input" type="text" aria-label="Title" placeholder="Raven crit build" bind:value={title} disabled={busy} />
    </label>
    <div class="field">
      <span class="label">Game</span>
      <GamePicker {serverId} value={game} onChange={(id) => { game = id; guessed = false }} addLabel={(n) => `+ Use '${n}' as a new game`} />
      {#if guessed}
        <p class="guess"><span class="hint">Game guessed from the link</span>
          <button type="button" class="btn btn-quiet keep" onclick={() => (guessed = false)}>Keep</button></p>
      {/if}
    </div>
    <label class="field">
      <span class="label">Build link</span>
      <input class="input" type="url" aria-label="Build link" placeholder="A video or a build planner (optional)" maxlength={MAX_LINK} bind:value={link} disabled={busy} />
    </label>
    <div class="field">
      <span class="label">Screenshots <span class="hint">Paste, drop or pick; drag to reorder</span></span>
      <ul class="shots">
        {#each shots as s, i (s.key)}
          <li draggable="true" class:dragging={dragging === i} ondragstart={(e) => { dragging = i; e.dataTransfer?.setData('text/plain', s.key) }}
            ondragover={(e) => dragOver(e, i)} ondragend={() => (dragging = null)}>
            <span class="pic">{#if shotSrc(s)}<img src={shotSrc(s)} alt="" draggable="false" />{/if}</span>
            <span class="name">{shotName(s)}</span>
            <span class="tools">
              <button type="button" class="icon-btn icon-btn-sm" aria-label="Move earlier: {shotName(s)}" disabled={i === 0} onclick={() => move(i, -1)}><Icon name="chevronLeft" size={12} /></button>
              <button type="button" class="icon-btn icon-btn-sm" aria-label="Move later: {shotName(s)}" disabled={i === shots.length - 1} onclick={() => move(i, 1)}><Icon name="chevronRight" size={12} /></button>
              <button type="button" class="icon-btn icon-btn-sm" aria-label="Remove {shotName(s)}" onclick={() => remove(i)}><Icon name="x" size={12} /></button>
            </span>
          </li>
        {/each}
        {#if shots.length < MAX_SHOTS}
          <li class="add"><button type="button" class="btn btn-quiet" onclick={() => void pick()}><Icon name="image" size={14} />Add screenshots</button></li>
        {/if}
      </ul>
      {#if shotsNote}<p class="shots-note" role="status">{shotsNote}</p>{/if}
    </div>
    <label class="field">
      <span class="label">Notes</span>
      <textarea class="input" aria-label="Notes" rows="5" placeholder="Modules, gear, how to play it (markdown works)" maxlength={MAX_NOTES} bind:value={notes} disabled={busy}></textarea>
    </label>
    {#if offerSorted}
      <label class="sorted"><input type="checkbox" name="sorted" bind:checked={sorted} disabled={busy} /> Mark as sorted <span class="hint">(no longer a quick drop)</span></label>
    {/if}
    {#if error}<p class="err" role="alert">{error}</p>{/if}
    <div class="foot">
      <button type="button" class="btn btn-lg btn-secondary" disabled={busy} onclick={cancel}>Cancel</button>
      <button type="button" class="btn btn-lg btn-primary save" disabled={!canSave} onclick={() => void save()}>{busy ? 'Saving…' : 'Save'}</button>
    </div>
  </div>
</div>

<style>
  .overlay { position: fixed; inset: 0; z-index: 30; }
  .modal { width: min(560px, 100%); }
  .foot { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: 8px; }
  .field { display: flex; flex-direction: column; gap: 4px; }
  .label { display: flex; align-items: baseline; gap: 8px; font-size: 12px; font-weight: 700; color: var(--text-2); }
  .counter { margin-left: auto; font-weight: 400; color: var(--text-3); font-variant-numeric: tabular-nums; }
  .counter.over { color: var(--danger-text); }
  .hint { font-weight: 400; color: var(--text-3); }
  textarea { resize: vertical; min-height: 96px; }
  .shots { list-style: none; margin: 0; padding: 0; display: grid; grid-template-columns: repeat(auto-fill, minmax(110px, 1fr)); gap: 8px; }
  .shots li { position: relative; display: flex; flex-direction: column; gap: 4px; padding: 4px; border-radius: var(--radius-sm); background: var(--bg-2); border: 1px solid var(--bg-3); cursor: grab; }
  .shots li.dragging { opacity: .5; }
  .shots li.add { cursor: default; align-items: center; justify-content: center; min-height: 90px; border-style: dashed; }
  .pic { aspect-ratio: 16 / 9; border-radius: var(--radius-xs); overflow: hidden; background: var(--bg-3); }
  .pic img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .name { font-size: 11px; color: var(--text-2); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .tools { display: flex; justify-content: flex-end; gap: 2px; }
  .shots-note { margin: 2px 0 0; font-size: 12px; color: var(--text-2); }
  .guess { margin: 2px 0 0; display: flex; align-items: center; gap: 6px; font-size: 13px; color: var(--text); }
  .sorted { display: flex; align-items: center; gap: 6px; font-size: 13px; color: var(--text); }
  .err { margin: 0; color: var(--danger-text); font-size: 13px; }
</style>
