<script lang="ts">
  // Settings › Chat › Clip drop (#382): offer new NVIDIA / Game Bar recordings as clips, and the folders it watches.
  // Its own feature, apart from Game activity (Settings › Privacy): it works with that off and shares nothing until
  // you share a clip. Windows only: elsewhere the section isn't shown (nothing is ever watched there).
  import Icon from './Icon.svelte'
  import { api, errorText } from '../lib/tauri'
  import { clipDropPrefs, setClipDropOn, setClipFolders } from '../lib/clipDropPrefs.svelte'
  import { isWindows } from '../lib/stream/platform'
  const uid = $props.id()

  const windows = isWindows()
  // the chosen folders, else the core's defaults under Videos (asked once)
  let defaultFolders = $state<string[]>([])
  $effect(() => {
    if (!windows) return
    let gone = false
    api.clipDropDefaults().then((d) => !gone && (defaultFolders = d), () => {})
    return () => (gone = true)
  })
  const folders = $derived(clipDropPrefs.folders ?? defaultFolders)
  let error = $state('')
  async function addFolder() {
    error = ''
    try {
      const got = await api.pickClipFolder()
      if (got && !folders.some((f) => f.toLowerCase() === got.toLowerCase())) setClipFolders([...folders, got])
    } catch (e) {
      error = errorText(e)
    }
  }
  const removeFolder = (f: string) => setClipFolders(folders.filter((x) => x !== f))
</script>

{#if windows}
  <div class="settings-field clip-drop">
    <h3 class="settings-section-title">Clip drop</h3>
    <label class="row">
      <input type="checkbox" role="switch" class="switch" data-switch="clip-drop" checked={clipDropPrefs.on} aria-describedby="{uid}-help"
        onchange={(e) => setClipDropOn(e.currentTarget.checked)} />
      <span>Offer new recordings as clips</span>
    </label>
    <small id="{uid}-help">When NVIDIA or Game Bar saves a recording in a watched folder, Pulse offers to share it to a clips channel or a DM. Nothing is sent unless you share it. Works without Game activity. This device only.</small>
    {#if clipDropPrefs.on}
      <div class="folders">
        <h4 class="title">Watched folders</h4>
        <p class="hint">New videos here and in their folders one level down (NVIDIA saves each game in its own) are offered.</p>
        {#if folders.length}
          <ul class="settings-list">
            {#each folders as f (f)}
              <li class="settings-list-item">
                <span class="who"><small class="path" data-tip={f}>{f}</small></span>
                <button type="button" class="btn btn-secondary" aria-label="Stop watching {f}" onclick={() => removeFolder(f)}>Remove</button>
              </li>
            {/each}
          </ul>
        {:else}
          <p class="hint">No folders: nothing is offered.</p>
        {/if}
        <div class="actions">
          <button type="button" class="btn btn-secondary" onclick={() => void addFolder()}><Icon name="plus" size={14} /> Add a folder</button>
          {#if clipDropPrefs.folders}
            <button type="button" class="btn btn-secondary" onclick={() => setClipFolders(null)}>Use the defaults</button>
          {/if}
        </div>
      </div>
    {/if}
    {#if error}<p class="error" role="alert">{error}</p>{/if}
  </div>
{/if}

<style>
  p { margin: 0; color: var(--text-2); font-size: 13px; }
  .row { display: flex; align-items: center; gap: 10px; font-weight: 600; }
  .folders { display: flex; flex-direction: column; gap: 6px; margin-left: 24px; }
  .title { margin: 8px 0 0; font-size: 13px; font-weight: 600; }
  .who { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .who small { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text-2); }
  .actions { display: flex; gap: 6px; }
  .error { margin: 0; color: var(--danger-text); font-size: 13px; }
</style>
