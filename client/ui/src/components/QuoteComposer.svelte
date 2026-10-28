<script lang="ts">
  // The quote composer: one compact row (text + "who said it?") that Enter posts, more lines on demand, image lines by
  // paste/drop/button. With `editing` it is the editor of an existing quote. The server is authoritative; its refusals
  // are shown as they come.
  //
  // The line text is a plain <textarea>, not the message composer's EmojiBox: quote lines are stored as plain text
  // (emoji typed or pasted work as characters) and EmojiBox's inline emoji images, mention popup and draft handling are
  // all things a quote line does not have.
  import { onMount, tick } from 'svelte'
  import { getCurrentWebview } from '@tauri-apps/api/webview'
  import { stillSrc } from '../lib/animate.svelte'
  import { mediaUrl } from '../lib/emoji'
  import { suggest } from '../lib/mentions'
  import { nameKey, stripMarks } from '../lib/quotes'
  import { displayName } from '../lib/selectors'
  import { app } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import type { Message } from '../lib/protocol/Message'
  import type { NewQuoteLine } from '../lib/protocol/NewQuoteLine'
  import Icon from './Icon.svelte'

  let {
    channelId,
    serverId,
    editing,
    onDone,
  }: {
    channelId: string
    serverId: string
    /** The quote being edited; absent when writing a new one. */
    editing?: Message
    onDone?: () => void
  } = $props()

  const MAX_LINES = 10
  const MAX_TEXT = 4000
  const MAX_NAME = 32
  const MAX_CAPTION = 200

  type Speaker = { id: string; name: string } | { id: null; name: string }
  type Picture = { hash: string; still_hash: string | null }
  type Row = {
    key: number
    id: string | null
    text: string
    caption: string
    pic: Picture | null
    speaker: Speaker | null
    /** What the speaker was when the line was loaded (edit mode). */
    was: string | null
    query: string
    open: boolean
    /** The highlighted entry of the picker list. */
    hi: number
    uploading: boolean
    error: string
  }

  let nextKey = 0
  const blank = (): Row => ({ key: nextKey++, id: null, text: '', caption: '', pic: null, speaker: null, was: null, query: '', open: false, hi: 0, uploading: false, error: '' })
  const speakerKey = (s: Speaker | null) => (s ? (s.id ? `u:${s.id}` : `n:${nameKey(s.name)}`) : null)

  function initial(): Row[] {
    const lines = editing?.quote?.lines
    if (!lines?.length) return [blank()]
    return lines.map((l) => {
      const speaker: Speaker | null = l.speaker_id ? { id: l.speaker_id, name: displayName(app.state, serverId, l.speaker_id) } : l.speaker_name ? { id: null, name: l.speaker_name } : null
      return {
        ...blank(), id: l.id, text: l.text ?? '', caption: l.caption ?? '', pic: l.media ? { hash: l.media.hash, still_hash: l.media.still_hash } : null,
        speaker, was: speakerKey(speaker),
      }
    })
  }
  let rows = $state<Row[]>(initial())
  let error = $state('')
  let busy = $state(false)
  // One id per quote being posted: kept across retries (a failed attempt may have reached the server) and renewed after success.
  let nonce: { id: string; sig: string } | null = null
  // Edit mode: the lines whose speaker changed while holding awards, waiting for a yes. Changing a speaker or adding or
  // removing a line takes the question down; a yes covers only the lines it named, at the counts it named (checked in
  // submit).
  let confirming = $state<{ key: number; awards: number; what: string }[]>([])
  // The quote as the store has it now: awards given while the editor is open count too. Opened from
  // its replies panel after it left the list (a jump), the panel's held copy is the live one.
  const live = $derived(
    editing &&
      (app.state.messages[channelId]?.find((m) => m.id === editing.id) ??
        (app.state.openRoot?.id === editing.id ? app.state.openRoot : editing)),
  )
  const awardsOf = (r: Row) => (r.id ? (live?.quote?.lines.find((l) => l.id === r.id)?.awards ?? 0) : 0)
  let root = $state<HTMLElement>()

  const members = $derived((app.state.members[serverId] ?? []).map((m) => ({ id: m.user.id, name: m.nickname ?? m.user.username })))

  onMount(() => {
    if (editing) focusText(rows[0].key)
  })

  const rowEl = (key: number) => root?.querySelector<HTMLElement>(`[data-key="${key}"]`)
  async function focusText(key: number) {
    await tick()
    rowEl(key)?.querySelector<HTMLElement>('.qtext, .caption')?.focus()
  }
  async function focusWho(key: number) {
    await tick()
    rowEl(key)?.querySelector<HTMLElement>('.who, .chip')?.focus()
  }

  function addLine() {
    if (rows.length >= MAX_LINES) return
    const r = blank()
    rows.push(r)
    confirming = []
    void focusText(r.key)
  }
  function removeLine(key: number) {
    if (rows.length > 1) rows = rows.filter((r) => r.key !== key)
    confirming = []
  }

  function choose(r: Row, s: Speaker) {
    r.speaker = s
    r.query = ''
    r.open = false
    error = ''
    confirming = []
    void focusText(r.key)
  }
  function clearSpeaker(r: Row) {
    r.speaker = null
    r.query = ''
    r.open = true
    confirming = []
    void focusWho(r.key)
  }
  const typed = (r: Row) => r.query.trim()
  /** The picker list: matching members, then the typed name as it is. */
  function options(r: Row): Speaker[] {
    const out: Speaker[] = suggest(members, typed(r)).map((m) => ({ id: m.id, name: m.name }))
    if (typed(r)) out.push({ id: null, name: typed(r) })
    return out
  }
  const label = (s: Speaker) => (s.id ? s.name : `Use “${s.name}” (not on Pulse yet)`)
  /** A typed name nobody picked: the member it exactly names (case and spacing aside), else that plain name. */
  function resolveTyped(r: Row) {
    const q = typed(r)
    if (r.speaker || !q) return
    const key = nameKey(q)
    const m = (app.state.members[serverId] ?? []).find((x) => nameKey(x.user.username) === key || nameKey(x.nickname ?? x.user.username) === key)
    r.speaker = m ? { id: m.user.id, name: m.nickname ?? m.user.username } : { id: null, name: q }
    r.query = ''
    r.open = false
    confirming = []
  }
  function whyKey(e: KeyboardEvent, r: Row) {
    const list = options(r)
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault()
      r.open = true
      if (list.length) r.hi = (r.hi + (e.key === 'ArrowDown' ? 1 : -1) + list.length) % list.length
    } else if (e.key === 'Enter') {
      e.preventDefault()
      e.stopPropagation()
      const pick = list[Math.min(r.hi, list.length - 1)]
      if (pick) choose(r, pick)
    } else if (e.key === 'Escape' && r.open) {
      e.stopPropagation()
      r.open = false
    }
  }

  // --- images ---
  const imageIn = (files: FileList | File[] | null | undefined) => [...(files ?? [])].find((f) => f.type.startsWith('image/'))
  function bytes(f: File): Promise<Uint8Array> {
    if (typeof f.arrayBuffer === 'function') return f.arrayBuffer().then((b) => new Uint8Array(b))
    return new Promise((res, rej) => {
      const fr = new FileReader()
      fr.onload = () => res(new Uint8Array(fr.result as ArrayBuffer))
      fr.onerror = () => rej(fr.error)
      fr.readAsArrayBuffer(f)
    })
  }
  /** Uploads an image to line `r`; `read` gives its bytes (a pasted or picked File, or a dropped path the core reads). */
  async function upload(r: Row, read: () => Promise<Uint8Array>) {
    if (r.uploading) return
    r.error = ''
    // typed words become the caption; if they don't fit, nothing is converted or thrown away. The boxes stay editable
    // while the image uploads, so the words are taken when it lands (if they grew past the limit meanwhile, posting says
    // so). A new image on an image line leaves its caption alone.
    const convert = !r.pic
    if (convert && [...r.text.trim()].length > MAX_CAPTION) {
      r.error = `That text is too long for a caption (${MAX_CAPTION} characters at most). Shorten it, or clear it first.`
      return
    }
    r.uploading = true
    try {
      const info = await api.uploadQuoteMedia(await read())
      // the image this one was replacing was removed meanwhile: the line is a text line again, so drop the upload
      if (!convert && !r.pic) return
      r.pic = { hash: info.hash, still_hash: info.still_hash }
      if (convert) {
        r.caption = r.text.trim()
        r.text = ''
      }
      void focusText(r.key)
    } catch (e) {
      // the line stays as it was, still editable; nothing is posted
      r.error = errorText(e)
    } finally {
      r.uploading = false
    }
  }
  function onPaste(e: ClipboardEvent, r: Row) {
    const f = imageIn(e.clipboardData?.files)
    if (!f) return
    e.preventDefault()
    void upload(r, () => bytes(f))
  }
  function onPick(e: Event, r: Row) {
    const input = e.currentTarget as HTMLInputElement
    const f = imageIn(input.files)
    input.value = ''
    if (f) void upload(r, () => bytes(f))
  }

  // Files dropped from the OS: Tauri takes them itself (its drag-drop handler is on, so the webview never sees an HTML5
  // drop carrying files) and reports the paths with a position in physical pixels. The line under that point lights up
  // while files are dragged over it and gets the first image dropped there. Every composer open (the new quote and an
  // edit) hears every drop; only the one whose line is under the pointer acts.
  let dropKey = $state<number | null>(null)
  function lineAt(pos: { x: number; y: number }): Row | undefined {
    const scale = window.devicePixelRatio || 1
    const el = document.elementFromPoint(pos.x / scale, pos.y / scale)?.closest<HTMLElement>('.qline')
    if (!el || !root?.contains(el)) return undefined
    return rows.find((r) => String(r.key) === el.dataset.key)
  }
  $effect(() => {
    let off: (() => void) | null = null
    let gone = false
    try {
      void getCurrentWebview()
        .onDragDropEvent((e) => {
          const p = e.payload
          if (p.type === 'enter' || p.type === 'over') dropKey = lineAt(p.position)?.key ?? null
          else if (p.type === 'leave') dropKey = null
          else if (p.type === 'drop') {
            dropKey = null
            const r = lineAt(p.position)
            if (r && p.paths.length) void upload(r, () => api.readQuoteImage(p.paths))
          }
        })
        .then((f) => (gone ? f() : (off = f)), () => {})
    } catch {
      // not in the app's webview (tests without it): no dropping
    }
    return () => {
      gone = true
      off?.()
      dropKey = null
    }
  })

  // --- posting ---
  const today = () => {
    const d = new Date()
    const p = (n: number) => String(n).padStart(2, '0')
    return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`
  }
  const isEmpty = (r: Row) => !r.pic && !r.text.trim() && !r.speaker && !r.query.trim() && !r.caption.trim()

  /** The lines to send, or null with `error` set. Wholly blank rows are skipped. */
  function build(): NewQuoteLine[] | null {
    const used = rows.filter((r) => !isEmpty(r))
    if (!used.length) {
      if (editing) error = 'Add the words or an image.'
      return null
    }
    const out: NewQuoteLine[] = []
    for (const r of used) {
      if (!r.speaker) {
        error = 'Who said it?'
        void focusWho(r.key)
        return null
      }
      if (r.speaker.id === null && ([...r.speaker.name].length > MAX_NAME)) {
        error = `A name can be at most ${MAX_NAME} characters.`
        return null
      }
      // The server refuses "." and ".." (and any name of only dots): a link to one could never be undone.
      if (r.speaker.id === null && /^\.+$/.test(nameKey(r.speaker.name))) {
        error = 'A name needs more than dots.'
        void focusWho(r.key)
        return null
      }
      const speaker = { speaker_id: r.speaker.id, speaker_name: r.speaker.id ? null : r.speaker.name }
      if (r.pic) {
        const caption = r.caption.trim()
        if ([...caption].length > MAX_CAPTION) {
          error = `A caption can be at most ${MAX_CAPTION} characters.`
          return null
        }
        out.push({ id: r.id, ...speaker, text: null, media_hash: r.pic.hash, caption: caption || null })
      } else {
        const text = stripMarks(r.text)
        if (!text) {
          error = 'Add the words or an image.'
          void focusText(r.key)
          return null
        }
        if ([...text].length > MAX_TEXT) {
          error = `A line can be at most ${MAX_TEXT} characters.`
          return null
        }
        out.push({ id: r.id, ...speaker, text, media_hash: null, caption: null })
      }
    }
    return out
  }

  async function submit(confirmed = false) {
    if (busy) return
    error = ''
    if (rows.some((r) => r.uploading)) {
      error = 'Wait for the image to finish uploading.'
      return
    }
    for (const r of rows) resolveTyped(r)
    const lines = build()
    if (!lines) return
    if (editing) {
      // "Save anyway" covers only the lines it named, and only at the counts it named
      const yes = confirmed ? confirming : []
      const risky = rows.filter((r) => awardsOf(r) > 0 && speakerKey(r.speaker) !== r.was)
      if (risky.some((r) => !yes.some((c) => c.key === r.key && c.awards >= awardsOf(r)))) {
        confirming = risky.map((r) => ({ key: r.key, awards: awardsOf(r), what: r.pic ? 'Image' : `“${[...r.text.trim()].slice(0, 30).join('')}”` }))
        return
      }
    }
    confirming = []
    busy = true
    try {
      if (editing) await api.editQuote(editing.id, { lines })
      else {
        // a retry of the same content keeps its id; changed content is a different request and gets its own
        const sig = JSON.stringify(lines)
        if (nonce?.sig !== sig) nonce = { id: crypto.randomUUID(), sig }
        await api.postQuote(channelId, { lines, today: today(), nonce: nonce.id })
      }
      nonce = null
      rows = [blank()]
      onDone?.()
      if (!editing) void focusText(rows[0].key)
    } catch (e) {
      error = errorText(e)
    } finally {
      busy = false
    }
  }

  function textKey(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
      e.preventDefault()
      void submit()
    }
  }
  function rootKey(e: KeyboardEvent) {
    if (e.key === 'Escape' && editing) {
      e.preventDefault()
      onDone?.()
    }
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="qc" bind:this={root} onkeydown={rootKey}>
  {#each rows as r (r.key)}
    <div class="qline" class:drop={dropKey === r.key} data-key={r.key}>
      <div class="row">
        {#if r.pic}
          <div class="pic">
            <img class="thumb" src={stillSrc({ hash: r.pic.hash, still_hash: r.pic.still_hash })} alt="" />
            <button type="button" class="x" aria-label="Remove image" onclick={() => (r.pic = null)}><Icon name="x" size={12} /></button>
          </div>
          <input class="caption" type="text" placeholder="Caption (optional)" aria-label="Caption" bind:value={r.caption} onkeydown={textKey}
            onpaste={(e) => onPaste(e, r)} />
        {:else}
          <textarea class="qtext" rows="1" placeholder={rows.length > 1 ? 'What they said' : 'Paste or type the quote'} aria-label="Quote text" bind:value={r.text}
            onkeydown={textKey} onpaste={(e) => onPaste(e, r)}></textarea>
        {/if}
        <div class="picker">
          {#if r.speaker}
            <button type="button" class="chip" title="Change who said it" onclick={() => clearSpeaker(r)}>— {r.speaker.name}</button>
          {:else}
            <input class="who" type="text" role="combobox" aria-expanded={r.open} aria-controls="opts-{r.key}" aria-activedescendant={r.open ? `opt-${r.key}-${r.hi}` : undefined}
              aria-autocomplete="list" aria-label="Who said it?" placeholder="who said it?" autocomplete="off" bind:value={r.query}
              onfocus={() => (r.open = true)} onblur={() => { r.open = false; resolveTyped(r) }} oninput={() => { r.open = true; r.hi = 0 }} onkeydown={(e) => whyKey(e, r)} />
            {#if r.open}
              <div class="opts" id="opts-{r.key}" role="listbox" aria-label="Members">
                {#each options(r) as o, i (o.id ?? 'plain')}
                  <button type="button" id="opt-{r.key}-{i}" role="option" aria-selected={i === r.hi} class:hi={i === r.hi} tabindex="-1"
                    onmousedown={(e) => e.preventDefault()} onclick={() => choose(r, o)}>{label(o)}</button>
                {/each}
              </div>
            {/if}
          {/if}
        </div>
        <button type="button" class="ibtn" title="Add an image" aria-label="Add image" onclick={() => rowEl(r.key)?.querySelector<HTMLInputElement>('input[type=file]')?.click()}><Icon name="image" size={16} /></button>
        <input type="file" accept="image/png,image/jpeg,image/gif,image/webp" hidden onchange={(e) => onPick(e, r)} />
        {#if rows.length > 1}
          <button type="button" class="ibtn" aria-label="Remove line" onclick={() => removeLine(r.key)}><Icon name="x" size={14} /></button>
        {/if}
      </div>
      {#if r.uploading}<p class="note">Uploading…</p>{/if}
      {#if r.error}<p class="err" role="alert">{r.error}</p>{/if}
    </div>
  {/each}
  {#if confirming.length}
    <div class="warn" role="alertdialog" aria-label="Change who said it?">
      {#each confirming as c (c.key)}
        <p><strong>{c.what}</strong> This line has {c.awards} {c.awards === 1 ? 'award' : 'awards'}. Changing who said it removes them.</p>
      {/each}
      <button type="button" class="confirm" onclick={() => void submit(true)}>Save anyway</button>
      <button type="button" onclick={() => (confirming = [])}>Keep editing</button>
    </div>
  {/if}
  {#if error}<p class="err" role="alert">{error}</p>{/if}
  <div class="foot">
    {#if rows.length < MAX_LINES}<button type="button" class="addline" onclick={addLine}>+ line</button>{/if}
    <span class="spacer"></span>
    {#if editing}<button type="button" onclick={() => onDone?.()}>Cancel</button>{/if}
    <button type="button" class="post" disabled={busy || rows.some((r) => r.uploading)} onclick={() => void submit()}>{editing ? 'Save' : 'Post'}</button>
  </div>
</div>

<style>
  .qc { flex-shrink: 0; margin: 0 18px 16px; padding: 8px; background: var(--bg-3); border-radius: 12px; display: flex; flex-direction: column; gap: 6px; }
  .row { display: flex; align-items: center; gap: 8px; }
  .qline { border-radius: 10px; }
  .qline.drop { outline: 2px dashed var(--accent); outline-offset: 2px; }
  .qtext, .caption, .who { min-width: 0; flex: 1; padding: 7px 10px; border: 1px solid transparent; border-radius: 8px; background: var(--bg-2); color: var(--text); font: inherit; resize: none; field-sizing: content; max-height: 140px; outline: none; }
  .qtext:focus, .caption:focus, .who:focus { border-color: var(--accent); }
  .picker { position: relative; width: 170px; flex-shrink: 0; }
  .who { width: 100%; flex: none; }
  .chip { max-width: 100%; height: 32px; padding: 0 10px; border: 0; border-radius: 8px; background: var(--quote-green-deep); color: var(--text); font: inherit; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .opts { position: absolute; bottom: calc(100% + 4px); left: 0; right: 0; z-index: 20; padding: 4px; background: #2e3037; border: 1px solid #3d4048; border-radius: 10px; display: flex; flex-direction: column; gap: 2px; font-size: 13px; }
  .opts button { min-height: 30px; padding: 0 8px; border: 0; border-radius: 6px; background: transparent; color: var(--text); text-align: left; font: inherit; }
  .opts button:hover, .opts button.hi { background: var(--bg-3); }
  .ibtn { width: 30px; height: 30px; flex-shrink: 0; display: grid; place-items: center; border: 0; border-radius: 8px; background: transparent; color: var(--text-3); cursor: pointer; }
  .ibtn:hover { background: var(--bg-2); color: var(--text); }
  .pic { position: relative; flex-shrink: 0; }
  .thumb { display: block; width: 44px; height: 44px; object-fit: cover; border-radius: 8px; }
  .x { position: absolute; top: -5px; right: -5px; width: 18px; height: 18px; display: grid; place-items: center; border: 0; border-radius: 50%; background: var(--bg-1, #1e1f24); color: var(--text); }
  .foot { display: flex; align-items: center; gap: 8px; }
  .spacer { flex: 1; }
  .foot button, .warn button { height: 28px; padding: 0 12px; border: 0; border-radius: 8px; background: var(--bg-2); color: var(--text); font: inherit; font-size: 13px; }
  .foot .post { background: var(--quote-green); color: var(--on-quote-green); font-weight: 700; }
  .foot .post:disabled { opacity: .5; }
  .warn { padding: 8px 10px; border-radius: 8px; background: var(--bg-2); display: flex; flex-wrap: wrap; align-items: center; gap: 8px; font-size: 13px; }
  .warn p { flex-basis: 100%; margin: 0; }
  .note { margin: 0; font-size: 12px; color: var(--text-3); }
  .err { margin: 0; font-size: 12px; color: var(--danger, #f0716b); }
</style>
