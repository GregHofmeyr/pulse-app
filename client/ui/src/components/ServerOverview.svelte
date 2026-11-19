<script lang="ts">
  // Server settings › Overview (#55), the first page: the server's icon and banner shown together as its invite card
  // shows them, then its name. Anyone in it changes them (the v1 trust model): the icon and the banner are each one
  // big clickable area with a pencil on hover, opening the same crop as your avatar (the icon, square) or profile banner
  // (the banner); Remove takes either away. Someone not in it only sees how it looks. Everyone in it sees a change as
  // it lands (`ServerUpdated`).
  import { onDestroy } from 'svelte'
  import CropDialog from './CropDialog.svelte'
  import Icon from './Icon.svelte'
  import ServerBanner from './ServerBanner.svelte'
  import ServerChip from './ServerChip.svelte'
  import { api, errorText } from '../lib/tauri'
  import { app } from '../lib/store.svelte'
  import { isMember } from '../lib/selectors'
  import type { CropRect } from '../lib/crop'
  import type { Server } from '../lib/protocol/Server'

  let { server }: { server: Server } = $props()

  const ACCEPT = 'image/png,image/jpeg,image/gif,image/webp'
  const canEdit = $derived(isMember(app.state, server.id))
  let cropping = $state<{ kind: 'avatar' | 'banner'; file: File } | null>(null)
  let busy = $state(false)
  let error = $state<string | null>(null)
  let iconInput = $state<HTMLInputElement>()
  let bannerInput = $state<HTMLInputElement>()
  // the name field: what's typed, until it's saved (a rename from elsewhere shows when nothing's typed)
  let typed = $state<string | null>(null)
  const name = $derived(typed ?? server.name)
  const changed = $derived(name.trim() !== '' && name.trim() !== server.name)
  let gone = false
  onDestroy(() => (gone = true))

  function pick(kind: 'avatar' | 'banner', e: Event & { currentTarget: HTMLInputElement }) {
    const file = e.currentTarget.files?.[0]
    e.currentTarget.value = '' // the same file again is a new pick
    if (file) cropping = { kind, file }
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
    const c = cropping
    cropping = null
    if (!c) return
    const id = server.id
    void run(async () => {
      const bytes = new Uint8Array(await c.file.arrayBuffer())
      if (c.kind === 'avatar') await api.setServerIcon(id, (await api.uploadAvatar(bytes, crop)).hash)
      else await api.setServerBanner(id, (await api.uploadBanner(bytes, crop)).hash)
    })
  }

  const removeIcon = () => void run(() => api.setServerIcon(server.id, null))
  const removeBanner = () => void run(() => api.setServerBanner(server.id, null))
  function save() {
    if (!changed) return
    const id = server.id
    const next = name.trim()
    void run(async () => {
      await api.renameServer(id, next)
      if (!gone) typed = null
    })
  }
  function nameKey(e: KeyboardEvent) {
    if (e.key === 'Enter') save()
    if (e.key === 'Escape' && typed !== null) {
      e.preventDefault()
      e.stopPropagation()
      typed = null
    }
  }
</script>

<section class="settings-pane">
  <h2>Overview</h2>
  <!-- the invite card's top: the banner, the icon over its bottom edge -->
  <div class="looks" aria-label="How {server.name} looks">
    {#if canEdit}
      <button type="button" class="pic banner edit" aria-label="Change banner" disabled={busy} onclick={() => bannerInput?.click()}>
        <ServerBanner id={server.id} hash={server.banner_hash} still={server.banner_still} />
        <span class="pencil" aria-hidden="true"><Icon name="pencil" size={20} /></span>
      </button>
      <button type="button" class="pic icon edit" aria-label="Change icon" disabled={busy} onclick={() => iconInput?.click()}>
        <ServerChip id={server.id} name={server.name} iconHash={server.icon_hash} still={server.icon_still} size={80} radius="24px" />
        <span class="pencil" aria-hidden="true"><Icon name="pencil" size={20} /></span>
      </button>
    {:else}
      <div class="pic banner"><ServerBanner id={server.id} hash={server.banner_hash} still={server.banner_still} /></div>
      <div class="pic icon"><ServerChip id={server.id} name={server.name} iconHash={server.icon_hash} still={server.icon_still} size={80} radius="24px" /></div>
    {/if}
  </div>
  {#if canEdit}
    <input bind:this={iconInput} class="file" type="file" accept={ACCEPT} aria-label="Upload icon" tabindex="-1" disabled={busy} onchange={(e) => pick('avatar', e)} />
    <input bind:this={bannerInput} class="file" type="file" accept={ACCEPT} aria-label="Upload banner" tabindex="-1" disabled={busy} onchange={(e) => pick('banner', e)} />
    <div class="row">
      <button type="button" class="btn btn-secondary" disabled={!server.icon_hash || busy} onclick={removeIcon}>Remove icon</button>
      <button type="button" class="btn btn-secondary" disabled={!server.banner_hash || busy} onclick={removeBanner}>Remove banner</button>
    </div>
    <label class="settings-field">
      <span class="settings-label">SERVER NAME</span>
      <span class="row">
        <input class="input grow" value={name} maxlength="64" aria-label="Server name" oninput={(e) => (typed = e.currentTarget.value)} onkeydown={nameKey} />
        <button type="button" class="btn btn-primary" disabled={!changed || busy} onclick={save}>Save</button>
      </span>
    </label>
    {#if busy}<span class="muted" role="status">Saving…</span>{/if}
  {:else}
    <p class="hint">Only people in {server.name} can change its icon, banner and name.</p>
  {/if}
  {#if error}<p class="error" role="alert">{error}</p>{/if}
</section>

{#if cropping}
  {#key cropping}
    <CropDialog file={cropping.file} kind={cropping.kind} onApply={upload} onCancel={() => (cropping = null)} />
  {/key}
{/if}

<style>
  .hint { margin: 0; font-size: 14px; color: var(--text-2); overflow-wrap: anywhere; }
  .looks { max-width: 440px; position: relative; padding-bottom: 44px; }
  .pic { position: relative; display: block; padding: 0; border: 0; background: none; overflow: hidden; }
  .banner { width: 100%; height: 140px; border-radius: var(--radius-lg); }
  /* the icon sits over the banner's bottom edge, ringed in the page's colour, as on the invite card */
  .icon { position: absolute; left: 16px; bottom: 0; width: 88px; height: 88px; padding: 4px; border-radius: 28px; background: var(--bg-1); }
  .icon :global(.chip) { display: grid; }
  .edit { cursor: pointer; }
  .edit:disabled { cursor: default; }
  .edit:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 2px; }
  /* the pencil, centred, over a dimmed picture: on hover and keyboard focus */
  .pencil { position: absolute; inset: 0; display: grid; place-items: center; background: var(--video-chrome); color: var(--on-video); opacity: 0; transition: opacity var(--dur-fast) var(--ease); }
  .icon .pencil { inset: 4px; border-radius: 24px; }
  .edit:not(:disabled):hover .pencil, .edit:focus-visible .pencil { opacity: 1; }
  @media (prefers-reduced-motion: reduce) { .pencil { transition: none; } }
  .row { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .grow { flex: 1; min-width: 0; max-width: 360px; }
  .file { position: absolute; width: 1px; height: 1px; opacity: 0; pointer-events: none; }
  .muted { color: var(--text-3); font-size: 13px; }
  .error { margin: 0; color: var(--danger-text); font-size: 13px; }
</style>
