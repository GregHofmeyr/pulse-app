<script lang="ts">
  // The quote composer: one compact row (text + "who said it?") that Enter posts, more lines on demand, image lines by
  // paste/drop/button. With `editing` it is the editor of an existing quote. The server is authoritative; its refusals
  // are shown as they come.
  //
  // The line text is a plain <textarea>, not the message composer's EmojiBox: quote lines are stored as plain text
  // (emoji typed or pasted work as characters) and EmojiBox's inline emoji images, mention popup and draft handling are
  // all things a quote line does not have.
  import { onMount, tick } from 'svelte'
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
    /** What the speaker was when the line was loaded (edit mode) and how many awards the line holds. */
    was: string | null
    awards: number
    query: string
    open: boolean
    uploading: boolean
    error: string
  }

  let nextKey = 0
  const blank = (): Row => ({ key: nextKey++, id: null, text: '', caption: '', pic: null, speaker: null, was: null, awards: 0, query: '', open: false, uploading: false, error: '' })
  const speakerKey = (s: Speaker | null) => (s ? (s.id ? `u:${s.id}` : `n:${nameKey(s.name)}`) : null)

  function initial(): Row[] {
    const lines = editing?.quote?.lines
    if (!lines?.length) return [blank()]
    return lines.map((l) => {
      const speaker: Speaker | null = l.speaker_id ? { id: l.speaker_id, name: displayName(app.state, serverId, l.speaker_id) } : l.speaker_name ? { id: null, name: l.speaker_name } : null
      return {
        ...blank(), id: l.id, text: l.text ?? '', caption: l.caption ?? '', pic: l.media ? { hash: l.media.hash, still_hash: l.media.still_hash } : null,
        speaker, was: speakerKey(speaker), awards: l.awards,
      }
    })
  }
  let rows = $state<Row[]>(initial())
  let error = $state('')
  let busy = $state(false)
  // Edit mode: the lines whose speaker changed while holding awards, waiting for a yes.
  let confirming = $state<{ key: number; awards: number }[]>([])
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
    void focusText(r.key)
  }
  function removeLine(key: number) {
    if (rows.length > 1) rows = rows.filter((r) => r.key !== key)
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
    void focusWho(r.key)
  }
  const matches = (r: Row) => suggest(members, r.query.trim())
  const typed = (r: Row) => r.query.trim().slice(0, MAX_NAME)
  function whyKey(e: KeyboardEvent, r: Row) {
    if (e.key === 'Enter') {
      e.preventDefault()
      e.stopPropagation()
      const m = matches(r)[0]
      if (m) choose(r, { id: m.id, name: m.name })
      else if (typed(r)) choose(r, { id: null, name: typed(r) })
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
  async function upload(r: Row, f: File) {
    if (r.uploading) return
    r.uploading = true
    r.error = ''
    try {
      const info = await api.uploadQuoteMedia(await bytes(f))
      r.pic = { hash: info.hash, still_hash: info.still_hash }
      r.text = ''
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
    void upload(r, f)
  }
  function onDrop(e: DragEvent, r: Row) {
    const f = imageIn(e.dataTransfer?.files)
    if (!f) return
    e.preventDefault()
    void upload(r, f)
  }
  function onPick(e: Event, r: Row) {
    const input = e.currentTarget as HTMLInputElement
    const f = imageIn(input.files)
    input.value = ''
    if (f) void upload(r, f)
  }

  // --- posting ---
  const today = () => {
    const d = new Date()
    const p = (n: number) => String(n).padStart(2, '0')
    return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`
  }
  const isEmpty = (r: Row) => !r.pic && !r.text.trim() && !r.speaker && !r.query.trim()

  /** The lines to send, or null with `error` set. Wholly blank rows are skipped. */
  function build(): NewQuoteLine[] | null {
    const used = rows.filter((r) => !isEmpty(r))
    if (!used.length) return null
    const out: NewQuoteLine[] = []
    for (const r of used) {
      if (!r.speaker) {
        error = 'Who said it?'
        void focusWho(r.key)
        return null
      }
      if (r.speaker.id === null && (r.speaker.name.length < 1 || r.speaker.name.length > MAX_NAME)) {
        error = `A name can be 1 to ${MAX_NAME} characters.`
        return null
      }
      const speaker = { speaker_id: r.speaker.id, speaker_name: r.speaker.id ? null : r.speaker.name }
      if (r.pic) {
        const caption = r.caption.trim()
        if (caption.length > MAX_CAPTION) {
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
    if (busy || rows.some((r) => r.uploading)) return
    error = ''
    const lines = build()
    if (!lines) return
    if (editing && !confirmed) {
      const hit = rows.filter((r) => r.id && r.awards > 0 && speakerKey(r.speaker) !== r.was)
      if (hit.length) {
        confirming = hit.map((r) => ({ key: r.key, awards: r.awards }))
        return
      }
    }
    confirming = []
    busy = true
    try {
      if (editing) await api.editQuote(editing.id, { lines })
      else await api.postQuote(channelId, { lines, today: today(), nonce: crypto.randomUUID() })
      rows = [blank()]
      onDone?.()
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
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="qline" data-key={r.key} ondragover={(e) => e.dataTransfer?.types.includes('Files') && e.preventDefault()} ondrop={(e) => onDrop(e, r)}>
      <div class="row">
        {#if r.pic}
          <div class="pic">
            <img class="thumb" src={stillSrc({ hash: r.pic.hash, still_hash: r.pic.still_hash })} alt="" />
            <button type="button" class="x" aria-label="Remove image" onclick={() => (r.pic = null)}><Icon name="x" size={12} /></button>
          </div>
          <input class="caption" type="text" maxlength={MAX_CAPTION} placeholder="Caption (optional)" aria-label="Caption" bind:value={r.caption} onkeydown={textKey} />
        {:else}
          <textarea class="qtext" rows="1" placeholder={rows.length > 1 ? 'What they said' : 'Paste or type the quote'} aria-label="Quote text" bind:value={r.text}
            onkeydown={textKey} onpaste={(e) => onPaste(e, r)}></textarea>
        {/if}
        <div class="picker">
          {#if r.speaker}
            <button type="button" class="chip" title="Change who said it" onclick={() => clearSpeaker(r)}>— {r.speaker.name}</button>
          {:else}
            <input class="who" type="text" role="combobox" aria-expanded={r.open} aria-controls="opts-{r.key}" aria-label="Who said it?" placeholder="who said it?" autocomplete="off"
              maxlength={MAX_NAME} bind:value={r.query} onfocus={() => (r.open = true)} onblur={() => (r.open = false)} oninput={() => (r.open = true)} onkeydown={(e) => whyKey(e, r)} />
            {#if r.open}
              <div class="opts" id="opts-{r.key}" role="listbox" aria-label="Members">
                {#each matches(r) as m (m.id)}
                  <button type="button" role="option" aria-selected="false" tabindex="-1" onmousedown={(e) => e.preventDefault()} onclick={() => choose(r, { id: m.id, name: m.name })}>{m.name}</button>
                {/each}
                {#if typed(r)}
                  <button type="button" role="option" aria-selected="false" tabindex="-1" onmousedown={(e) => e.preventDefault()} onclick={() => choose(r, { id: null, name: typed(r) })}>Use “{typed(r)}” (not on Pulse yet)</button>
                {/if}
              </div>
            {/if}
          {/if}
        </div>
        <label class="ibtn" title="Add an image" aria-label="Add image">
          <Icon name="image" size={16} />
          <input type="file" accept="image/png,image/jpeg,image/gif,image/webp" hidden onchange={(e) => onPick(e, r)} />
        </label>
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
        <p>This line has {c.awards} {c.awards === 1 ? 'award' : 'awards'}. Changing who said it removes them.</p>
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
    <button type="button" class="post" disabled={busy} onclick={() => void submit()}>{editing ? 'Save' : 'Post'}</button>
  </div>
</div>

<style>
  .qc { flex-shrink: 0; margin: 0 18px 16px; padding: 8px; background: var(--bg-3); border-radius: 12px; display: flex; flex-direction: column; gap: 6px; }
  .row { display: flex; align-items: center; gap: 8px; }
  .qtext, .caption, .who { min-width: 0; flex: 1; padding: 7px 10px; border: 1px solid transparent; border-radius: 8px; background: var(--bg-2); color: var(--text); font: inherit; resize: none; field-sizing: content; max-height: 140px; outline: none; }
  .qtext:focus, .caption:focus, .who:focus { border-color: var(--accent); }
  .picker { position: relative; width: 170px; flex-shrink: 0; }
  .who { width: 100%; flex: none; }
  .chip { max-width: 100%; height: 32px; padding: 0 10px; border: 0; border-radius: 8px; background: var(--quote-green-deep); color: var(--text); font: inherit; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .opts { position: absolute; bottom: calc(100% + 4px); left: 0; right: 0; z-index: 20; padding: 4px; background: #2e3037; border: 1px solid #3d4048; border-radius: 10px; display: flex; flex-direction: column; gap: 2px; font-size: 13px; }
  .opts button { min-height: 30px; padding: 0 8px; border: 0; border-radius: 6px; background: transparent; color: var(--text); text-align: left; font: inherit; }
  .opts button:hover { background: var(--bg-3); }
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
