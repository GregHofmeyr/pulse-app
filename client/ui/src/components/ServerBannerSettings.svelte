<script lang="ts">
  // Server settings → Banner (#475), its owner's only: the picture its invite cards show over its chip (and the invite
  // page too). Upload crops it as a profile banner is cropped (the same upload, limits and still), then sets it; Remove
  // goes back to the gradient of the server's colours. Everyone sees the change as it lands (`ServerUpdated`).
  import { onDestroy } from 'svelte'
  import CropDialog from './CropDialog.svelte'
  import ServerBanner from './ServerBanner.svelte'
  import ServerChip from './ServerChip.svelte'
  import { api, errorText } from '../lib/tauri'
  import type { CropRect } from '../lib/crop'
  import type { Server } from '../lib/protocol/Server'

  let { server }: { server: Server } = $props()

  const ACCEPT = 'image/png,image/jpeg,image/gif,image/webp'
  let cropping = $state<File | null>(null)
  let busy = $state(false)
  let error = $state<string | null>(null)
  let gone = false
  onDestroy(() => (gone = true))

  function pick(e: Event & { currentTarget: HTMLInputElement }) {
    const file = e.currentTarget.files?.[0]
    e.currentTarget.value = '' // the same file again is a new pick
    if (file) cropping = file
  }

  async function run(work: () => Promise<unknown>) {
    if (busy) return
    busy = true
    error = null
    try {
      await work()
    } catch (e) {
      if (!gone) error = errorText(e)
    } finally {
      if (!gone) busy = false
    }
  }

  function upload(crop: CropRect) {
    const file = cropping
    cropping = null
    if (!file) return
    const id = server.id
    void run(async () => {
      const info = await api.uploadBanner(new Uint8Array(await file.arrayBuffer()), crop)
      await api.setServerBanner(id, info.hash)
    })
  }

  const remove = () => void run(() => api.setServerBanner(server.id, null))
</script>

<section class="settings-pane">
  <h2>Banner</h2>
  <p class="hint">Shown across the top of {server.name}'s invites. Without one, they use a gradient of its colours.</p>
  <div class="preview" aria-label="Banner preview">
    <div class="pic"><ServerBanner id={server.id} hash={server.banner_hash} still={server.banner_still} /></div>
    <span class="chip"><ServerChip id={server.id} name={server.name} iconHash={server.icon_hash} size={48} /></span>
  </div>
  <div class="row">
    <label class="btn btn-secondary upload" class:disabled={busy}>Upload
      <input class="file" type="file" accept={ACCEPT} aria-label="Upload banner" disabled={busy} onchange={pick} />
    </label>
    <button type="button" class="btn btn-secondary" disabled={!server.banner_hash || busy} onclick={remove}>Remove</button>
    {#if busy}<span class="muted" role="status">Saving…</span>{/if}
  </div>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
</section>

{#if cropping}
  {#key cropping}
    <CropDialog file={cropping} kind="banner" onApply={upload} onCancel={() => (cropping = null)} />
  {/key}
{/if}

<style>
  .hint { margin: 0 0 16px; font-size: 14px; color: var(--text-2); overflow-wrap: anywhere; }
  /* the invite card's top in miniature: the banner, the chip over its bottom edge */
  .preview { max-width: 440px; margin-bottom: 40px; position: relative; }
  .pic { height: 120px; border-radius: var(--radius-lg); overflow: hidden; }
  .chip { position: absolute; left: 12px; bottom: -28px; padding: 3px; border-radius: 32%; background: var(--bg-1); display: inline-grid; }
  .row { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .upload { position: relative; cursor: pointer; }
  .upload:focus-within { outline: 2px solid var(--focus-ring); outline-offset: 2px; }
  .upload.disabled { opacity: .5; cursor: default; }
  .file { position: absolute; width: 1px; height: 1px; opacity: 0; pointer-events: none; }
  .muted { color: var(--text-3); font-size: 13px; }
  .error { margin: 8px 0 0; color: var(--danger-text); font-size: 13px; }
</style>
