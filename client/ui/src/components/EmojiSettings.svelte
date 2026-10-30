<script lang="ts">
  import { app } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import { defaultName, isValidEmojiName, mediaUrl } from '../lib/emoji'
  import { displayName } from '../lib/selectors'
  import type { Server } from '../lib/protocol/Server'
  import type { MediaInfo } from '../lib/protocol/MediaInfo'

  let { server }: { server: Server } = $props()

  const MAX = 100
  const emoji = $derived(
    Object.values(app.state.customEmoji)
      .filter((e) => e.server_id === server.id && e.hash !== null)
      .sort((a, b) => a.name.localeCompare(b.name)),
  )

  let error = $state('')
  let busy = $state(false)

  // Add flow: upload first (the server processes it), preview the result, then name and save.
  let uploaded = $state<MediaInfo | null>(null)
  let newName = $state('')

  /** Shown, and logged (the reason, not the file or name): a failure must never be silent. */
  function fail(what: string, message: string) {
    error = message
    void api.uiLog(`emoji: ${what}: ${message}`)
  }

  async function pick(e: Event) {
    const input = e.currentTarget as HTMLInputElement
    const file = input.files?.[0]
    input.value = ''
    if (!file) return
    error = ''
    busy = true
    try {
      uploaded = await api.uploadEmojiMedia(new Uint8Array(await file.arrayBuffer()))
      newName = defaultName(file.name)
    } catch (err) {
      uploaded = null
      fail('upload failed', errorText(err))
    } finally {
      busy = false
    }
  }

  const NAME_RULE = 'Emoji names are 2-32 letters, numbers or underscores (no spaces or dashes).'
  const PREVIEW_FAILED = "The image uploaded, but its preview couldn't be shown. Cancel and try again, or try another image."
  const FULL = `This server has ${MAX} emoji, the most it can hold; delete one first.`
  const validName = $derived(isValidEmojiName(newName.trim()))
  /** Typing again: a refused-name alert is stale (the hint under the field keeps saying it while it's still wrong). */
  function edited() {
    if (error === NAME_RULE) error = ''
  }
  function cancelAdd() {
    uploaded = null
    error = ''
  }

  async function save() {
    if (!uploaded || busy) return
    if (!validName) return fail('name refused', NAME_RULE)
    error = ''
    busy = true
    try {
      await api.createEmoji(server.id, newName.trim(), uploaded.hash)
      uploaded = null
    } catch (err) {
      fail('save failed', errorText(err))
    } finally {
      busy = false
    }
  }

  let renameBusy = false // apart from `busy`, so a rename and an upload/save don't block each other
  let renaming = $state<{ id: string; name: string } | null>(null)
  async function commitRename() {
    if (!renaming || renameBusy) return
    if (!isValidEmojiName(renaming.name.trim())) return fail('name refused', NAME_RULE)
    const { id, name } = renaming
    error = ''
    renameBusy = true
    try {
      await api.renameEmoji(id, name.trim())
      renaming = null
    } catch (err) {
      fail('rename failed', errorText(err))
    } finally {
      renameBusy = false
    }
  }
  const focus = (n: HTMLElement) => n.focus()

  let confirming = $state<string | null>(null)
  let deleteBusy = false
  async function remove(id: string) {
    if (deleteBusy) return
    error = ''
    deleteBusy = true
    try {
      await api.deleteEmoji(id)
      confirming = null
    } catch (err) {
      fail('delete failed', errorText(err))
    } finally {
      deleteBusy = false
    }
  }

  const when = (iso: string) => new Date(iso).toLocaleDateString()
</script>

<section>
  <header>
    <h2>Emoji</h2>
    <span class="count" aria-label="Emoji used">{emoji.length} / {MAX}</span>
  </header>
  <p class="hint">Anyone in the server can add, rename and delete emoji. PNG, JPEG, GIF or WebP, up to 2 MB.</p>

  {#if error}<p class="error" role="alert">{error}</p>{/if}

  {#if uploaded}
    <div class="add">
      <div class="previews">
        <img src={mediaUrl(uploaded.hash)} alt="Preview at 128 pixels" width="128" height="128"
          onerror={() => fail('preview failed', PREVIEW_FAILED)} />
        <img src={mediaUrl(uploaded.hash)} alt="Preview at 32 pixels" width="32" height="32" />
      </div>
      <label>Name
        <input bind:value={newName} maxlength="32" aria-label="Emoji name" aria-describedby="emoji-name-hint" aria-invalid={!validName}
          oninput={edited} onkeydown={(e) => { if (e.key === 'Enter') void save(); else if (e.key === 'Escape') { e.preventDefault(); cancelAdd() } }} />
      </label>
      <!-- always present (emptied when fine): a live region added along with its text often isn't announced -->
      <small id="emoji-name-hint" class="name-hint" aria-live="polite">{validName ? '' : NAME_RULE}</small>
      <div class="row">
        <button class="primary" disabled={busy} onclick={save}>Save</button>
        <button onclick={cancelAdd}>Cancel</button>
      </div>
    </div>
  {:else}
    <label class="file" class:off={busy || emoji.length >= MAX}>
      {busy ? 'Uploading…' : 'Add emoji'}
      <input type="file" accept="image/png,image/jpeg,image/gif,image/webp" disabled={busy || emoji.length >= MAX} onchange={pick} />
    </label>
    {#if emoji.length >= MAX}<p class="full">{FULL}</p>{/if}
  {/if}

  {#if emoji.length === 0}
    <p class="empty">No custom emoji yet.</p>
  {:else}
    <ul>
      {#each emoji as e (e.id)}
        <li>
          <img src={mediaUrl(e.still_hash ?? e.hash!)} alt={e.name} width="32" height="32" />
          <div class="meta">
            {#if renaming?.id === e.id}
              <input class="rename" aria-label="Rename emoji" bind:value={renaming.name} maxlength="32" use:focus
                aria-describedby="emoji-rename-hint" aria-invalid={!isValidEmojiName(renaming.name.trim())} oninput={edited}
                onkeydown={(ev) => { if (ev.key === 'Enter') void commitRename(); else if (ev.key === 'Escape') { ev.preventDefault(); renaming = null; error = '' } }} />
            {:else}
              <span class="name">:{e.name}:</span>
            {/if}
            {#if renaming?.id === e.id}
              <small id="emoji-rename-hint" class="name-hint" aria-live="polite">{isValidEmojiName(renaming.name.trim()) ? '' : NAME_RULE}</small>
            {/if}
            <small>{e.added_by ? displayName(app.state, server.id, e.added_by) : 'Unknown'} · {when(e.created_at)}</small>
          </div>
          {#if confirming === e.id}
            <span class="q">Delete :{e.name}:?</span>
            <button class="danger" onclick={() => remove(e.id)}>Delete</button>
            <button onclick={() => { confirming = null; error = '' }}>Cancel</button>
          {:else}
            <button aria-label="Rename :{e.name}:" onclick={() => (renaming = { id: e.id, name: e.name })}>Rename</button>
            <button aria-label="Delete :{e.name}:" onclick={() => (confirming = e.id)}>Delete</button>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  header { display: flex; align-items: baseline; gap: 12px; }
  h2 { margin: 0 0 8px; font-size: 20px; }
  .count { color: var(--text-3); font-size: 13px; }
  .hint { margin: 0 0 16px; color: var(--text-3); font-size: 13px; }
  .error { margin: 0 0 12px; color: #f2616b; font-size: 13px; }
  .empty { color: var(--text-3); }
  button { height: 32px; padding: 0 12px; border: 1px solid var(--bg-4); border-radius: 8px; background: transparent; color: var(--text); font-size: 13px; }
  button:hover { background: var(--bg-3); }
  button.danger { color: #f2616b; }
  button.primary { background: var(--accent); border-color: transparent; color: var(--on-accent); font-weight: 600; }
  .file { display: inline-flex; align-items: center; height: 36px; padding: 0 16px; margin-bottom: 20px; border-radius: 8px; background: var(--bg-3); font-weight: 600; font-size: 13px; cursor: pointer; }
  .file.off { opacity: .5; cursor: default; }
  .file input { position: absolute; width: 1px; height: 1px; opacity: 0; }
  .file:focus-within { outline: 2px solid var(--text-3); }
  .add { display: flex; flex-direction: column; gap: 12px; margin-bottom: 20px; padding: 16px; border: 1px solid var(--bg-4); border-radius: 12px; }
  .previews { display: flex; align-items: flex-end; gap: 16px; }
  .add label { display: flex; flex-direction: column; gap: 4px; font-size: 12px; color: var(--text-3); }
  input:not([type='file']) { height: 34px; padding: 0 10px; border: 1px solid var(--bg-4); border-radius: 8px; background: var(--bg-0); color: var(--text); }
  .row { display: flex; gap: 8px; }
  .name-hint { color: #f2616b; font-size: 12px; }
  /* empty but kept (a live region must exist before its text): take back the flex gap */
  .add .name-hint:empty { margin-top: -12px; }
  .full { margin: -12px 0 20px; color: var(--text-3); font-size: 13px; }
  ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
  li { display: flex; align-items: center; gap: 12px; padding: 8px 10px; border-radius: 10px; background: var(--bg-0); }
  li img { object-fit: contain; }
  .meta { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .name { font-weight: 600; }
  small { color: var(--text-3); }
  .q { font-size: 13px; color: var(--text-2); }
</style>
