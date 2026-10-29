<script lang="ts">
  // Settings → Profile (spec §4): settings on the left, a live preview on the right. Everything,
  // uploads included, is staged in the draft until Save sends it in one request; the store isn't
  // touched before then, so leaving without saving leaves you exactly as you were.
  import { onDestroy, onMount } from 'svelte'
  import ProfileCard from './ProfileCard.svelte'
  import Avatar from './Avatar.svelte'
  import Name from './Name.svelte'
  import CropDialog from './CropDialog.svelte'
  import EmojiBox from './EmojiBox.svelte'
  import Segmented from './Segmented.svelte'
  import { app, loadProfile } from '../lib/store.svelte'
  import { setProfile } from '../lib/state'
  import { api, errorText } from '../lib/tauri'
  import { draftFrom, isDirty, previewCard, previewUser, toRequest, type Draft } from '../lib/profileDraft'
  import { presetFor, THEME_PRESETS } from '../lib/cardTheme'
  import { NAME_FONTS } from '../lib/nameFonts'
  import { lightenForName, NAME_GRADIENTS, NAME_SOLIDS, type NamePreset } from '../lib/nameStyle'
  import type { CropRect } from '../lib/crop'
  import type { Profile } from '../lib/protocol/Profile'

  let { dirty = $bindable(false) }: { dirty?: boolean } = $props()

  type Kind = 'avatar' | 'banner'
  const ACCEPT = 'image/png,image/jpeg,image/gif,image/webp'
  const BIO_MAX = 190
  const SAVED_MS = 2000

  let profile = $state.raw<Profile | null>(null)
  let saved = $state.raw<Draft | null>(null)
  let draft = $state<Draft | null>(null)
  let loadError = $state<string | null>(null)
  // "Custom" picked while the colours still match a preset: show the pickers anyway.
  let customTheme = $state(false)
  let customColour = $state(false)
  let cropping = $state<{ file: File; kind: Kind } | null>(null)
  let uploading = $state<Record<Kind, boolean>>({ avatar: false, banner: false })
  let uploadError = $state<Record<Kind, string | null>>({ avatar: null, banner: null })
  let saving = $state(false)
  let saveError = $state<string | null>(null)
  let justSaved = $state(false)
  let savedTimer: ReturnType<typeof setTimeout> | undefined
  let bioBox = $state<EmojiBox>()
  let gone = false

  const me = $derived(app.state.me)
  const pu = $derived(me && draft ? previewUser(me, draft) : null)
  const pc = $derived(draft ? previewCard(draft) : null)
  const changed = $derived(!!draft && !!saved && isDirty(draft, saved))
  $effect(() => {
    dirty = changed
  })

  const themeKey = $derived(draft && !customTheme ? presetFor(draft.themeTop, draft.themeBottom) : null)
  const same = (p: NamePreset) => draft !== null && p.colors.join() === draft.colors.join()
  const colourKey = $derived.by(() => {
    if (!draft || customColour) return 'custom'
    if (draft.colors.length === 0) return 'default'
    const solid = NAME_SOLIDS.find(same)
    if (solid) return `solid:${solid.key}`
    const gradient = NAME_GRADIENTS.find(same)
    return gradient ? `gradient:${gradient.key}` : 'custom'
  })
  const lightened = $derived(colourKey === 'custom' && !!draft?.colors.some((c) => lightenForName(c) !== c.toLowerCase()))
  // Counted as the server counts: characters, after trailing whitespace is trimmed.
  const bioLength = $derived(draft ? [...draft.bio.trimEnd()].length : 0)
  const bioOver = $derived(bioLength > BIO_MAX)
  const busy = $derived(uploading.avatar || uploading.banner)

  function take(p: Profile) {
    profile = p
    saved = draftFrom(p)
    draft = draftFrom(p)
    customTheme = false
    customColour = false
  }

  onMount(() => {
    if (!me) return
    loadProfile(me.id).then(
      (p) => !gone && take(p),
      (e) => !gone && (loadError = errorText(e)),
    )
  })
  onDestroy(() => {
    gone = true
    clearTimeout(savedTimer)
  })

  function pickFile(kind: Kind, e: Event & { currentTarget: HTMLInputElement }) {
    const file = e.currentTarget.files?.[0]
    e.currentTarget.value = '' // the same file again is a new pick
    if (file) cropping = { file, kind }
  }

  /** Upload the cropped picture and stage it. A refusal leaves the draft as it was. */
  async function upload(crop: CropRect) {
    const pick = cropping
    cropping = null
    if (!pick) return
    const { file, kind } = pick
    uploadError[kind] = null
    uploading[kind] = true
    try {
      const bytes = new Uint8Array(await file.arrayBuffer())
      const info = await (kind === 'avatar' ? api.uploadAvatar(bytes, crop) : api.uploadBanner(bytes, crop))
      if (!draft) return
      if (kind === 'avatar') {
        draft.avatarHash = info.hash
        draft.avatarStill = info.still_hash
      } else {
        draft.bannerHash = info.hash
        draft.bannerStill = info.still_hash
      }
    } catch (e) {
      uploadError[kind] = errorText(e)
    } finally {
      uploading[kind] = false
    }
  }

  function remove(kind: Kind) {
    if (!draft) return
    uploadError[kind] = null
    if (kind === 'avatar') draft.avatarHash = draft.avatarStill = null
    else draft.bannerHash = draft.bannerStill = null
  }

  function setTheme(top: string, bottom: string) {
    if (!draft) return
    customTheme = false
    draft.themeTop = top
    draft.themeBottom = bottom
  }

  function setColours(colors: string[], custom = false) {
    if (!draft) return
    customColour = custom
    draft.colors = [...colors]
  }
  function pickCustomColour() {
    if (!draft) return
    setColours(draft.colors.length ? draft.colors : ['#ffffff'], true)
  }
  function setCustomCount(kind: 'solid' | 'gradient') {
    if (!draft) return
    const [a, b] = draft.colors
    setColours(kind === 'solid' ? [a] : [a, b ?? a], true)
  }

  function showBio(text: string) {
    bioBox?.setText({ text, caret: text.length }, false)
  }

  function reset() {
    if (!profile) return
    take(profile)
    uploadError = { avatar: null, banner: null }
    saveError = null
    showBio(saved!.bio)
  }

  async function save() {
    if (!draft || saving || !changed || bioOver || busy) return
    saving = true
    saveError = null
    try {
      const answer = await api.updateProfile(toRequest(draft))
      if (gone) return
      app.state = setProfile(app.state, answer)
      take(answer)
      showBio(answer.card.bio ?? '')
      justSaved = true
      clearTimeout(savedTimer)
      savedTimer = setTimeout(() => (justSaved = false), SAVED_MS)
    } catch (e) {
      saveError = errorText(e)
    } finally {
      saving = false
    }
  }
</script>

{#snippet picture(kind: Kind, title: string, hash: string | null)}
  <div class="settings-field {kind}-field">
    <span class="settings-label">{title}</span>
    <div class="row">
      <label class="btn" class:disabled={uploading[kind]}>Upload
        <input class="file" type="file" accept={ACCEPT} aria-label="Upload {kind}" disabled={uploading[kind]} onchange={(e) => pickFile(kind, e)} />
      </label>
      <button type="button" class="btn" disabled={!hash || uploading[kind]} onclick={() => remove(kind)}>Remove</button>
      {#if uploading[kind]}<span class="muted" role="status">Uploading…</span>{/if}
    </div>
    {#if uploadError[kind]}<p class="error" role="alert">{uploadError[kind]}</p>{/if}
  </div>
{/snippet}

<section class="settings-pane">
  <h2>Profile</h2>
  {#if loadError}
    <p class="error" role="alert">{loadError}</p>
  {:else if !draft || !me || !pu || !pc || !profile}
    <p class="muted">Loading…</p>
  {:else}
    <div class="layout">
      <div class="settings">
        {@render picture('avatar', 'Avatar', draft.avatarHash)}
        {@render picture('banner', 'Banner', draft.bannerHash)}

        <div class="settings-field theme">
          <span class="settings-label">Card theme</span>
          <div class="swatches" role="radiogroup" aria-label="Card theme">
            {#each THEME_PRESETS as p (p.key)}
              <button type="button" role="radio" class="swatch" aria-label={p.label} title={p.label} aria-checked={themeKey === p.key}
                style:background="linear-gradient({p.top}, {p.bottom})" onclick={() => setTheme(p.top, p.bottom)}></button>
            {/each}
            <button type="button" role="radio" class="chip" aria-checked={themeKey === null} onclick={() => (customTheme = true)}>Custom</button>
          </div>
          {#if themeKey === null}
            <div class="row">
              <input type="color" aria-label="Top colour" value={draft.themeTop} oninput={(e) => (draft!.themeTop = e.currentTarget.value)} />
              <input type="color" aria-label="Bottom colour" value={draft.themeBottom} oninput={(e) => (draft!.themeBottom = e.currentTarget.value)} />
            </div>
          {/if}
        </div>

        <div class="settings-field">
          <span class="settings-label">Name font</span>
          <div class="chips" role="radiogroup" aria-label="Name font">
            <button type="button" role="radio" class="chip" aria-checked={draft.font === null} onclick={() => (draft!.font = null)}>Default</button>
            {#each NAME_FONTS as f (f.key)}
              <button type="button" role="radio" class="chip" aria-checked={draft.font === f.key}
                style="font-family: {f.family}; font-weight: {f.weight}; font-size: calc(13px * {f.scale})" onclick={() => (draft!.font = f.key)}>{f.label}</button>
            {/each}
          </div>
        </div>

        <div class="settings-field colour">
          <span class="settings-label">Name colour</span>
          <div class="swatches" role="radiogroup" aria-label="Name colour">
            <button type="button" role="radio" class="chip" aria-checked={colourKey === 'default'} onclick={() => setColours([])}>Default</button>
            {#each NAME_SOLIDS as p (p.key)}
              <button type="button" role="radio" class="swatch round" aria-label={p.label} title={p.label} aria-checked={colourKey === `solid:${p.key}`}
                style:background={p.colors[0]} onclick={() => setColours(p.colors)}></button>
            {/each}
            {#each NAME_GRADIENTS as p (p.key)}
              <button type="button" role="radio" class="swatch round" aria-label="{p.label} gradient" title="{p.label} gradient" aria-checked={colourKey === `gradient:${p.key}`}
                style:background="linear-gradient(135deg, {p.colors[0]}, {p.colors[1]})" onclick={() => setColours(p.colors)}></button>
            {/each}
            <button type="button" role="radio" class="chip" aria-checked={colourKey === 'custom'} onclick={pickCustomColour}>Custom</button>
          </div>
          {#if colourKey === 'custom'}
            <Segmented label="Custom colour" value={draft.colors.length >= 2 ? 'gradient' : 'solid'} onchange={setCustomCount}
              options={[{ value: 'solid', label: 'Solid' }, { value: 'gradient', label: 'Gradient' }]} />
            <div class="row">
              {#each draft.colors as c, i}
                <input type="color" aria-label={draft.colors.length >= 2 ? (i === 0 ? 'First colour' : 'Second colour') : 'Colour'} value={c}
                  oninput={(e) => (draft!.colors[i] = e.currentTarget.value)} />
              {/each}
            </div>
            {#if lightened}<small>Lightened so it reads on dark backgrounds.</small>{/if}
          {/if}
        </div>

        <div class="settings-field">
          <span class="settings-label">Pronouns</span>
          <input class="text" type="text" aria-label="Pronouns" maxlength="40" bind:value={draft.pronouns} />
        </div>

        <div class="settings-field">
          <span class="settings-label">Bio</span>
          <EmojiBox bind:this={bioBox} bind:text={draft.bio} label="Bio" class="bio" />
          <small class="counter" class:over={bioOver}>{bioLength}/{BIO_MAX}</small>
        </div>
      </div>

      <div class="preview" aria-label="Preview">
        <ProfileCard user={pu} card={pc} badges={profile.badges} stats={profile.stats} serverId={null} preview />
        <div class="line">
          <Avatar id={me.id} name={me.username} user={pu} size={36} />
          <div class="said">
            <Name userId={me.id} serverId={null} user={pu} text={me.username} interactive={false} />
            <p>Looking good!</p>
          </div>
        </div>
      </div>
    </div>

    <div class="footer">
      {#if saveError}<p class="error" role="alert">{saveError}</p>{/if}
      <span class="state">
        {#if changed}Unsaved changes{:else if justSaved}<span class="ok" role="status">Saved</span>{/if}
      </span>
      <button type="button" class="btn" disabled={!changed || saving} onclick={reset}>Reset</button>
      <button type="button" class="btn primary" disabled={!changed || saving || bioOver || busy} onclick={() => void save()}>Save</button>
    </div>
  {/if}
</section>

{#if cropping}
  {#key cropping}
    <CropDialog file={cropping.file} kind={cropping.kind} onApply={(c) => void upload(c)} onCancel={() => (cropping = null)} />
  {/key}
{/if}

<style>
  .layout { display: grid; grid-template-columns: minmax(0, 1fr) 300px; gap: 24px; align-items: start; }
  .settings { display: flex; flex-direction: column; gap: 20px; min-width: 0; }
  .preview { position: sticky; top: 0; display: flex; flex-direction: column; gap: 16px; }
  @media (max-width: 860px) {
    .layout { grid-template-columns: minmax(0, 1fr); }
    .preview { position: static; order: -1; }
  }
  .row { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .btn { display: inline-flex; align-items: center; height: 34px; padding: 0 14px; border: 1px solid var(--bg-4); border-radius: 10px; background: transparent; color: var(--text); font-weight: 600; cursor: pointer; }
  .btn:disabled, .btn.disabled { opacity: .5; cursor: default; }
  .btn.primary { border-color: transparent; background: var(--accent); color: var(--on-accent); }
  /* the file input sits under its label, which is the button people see */
  label.btn { position: relative; }
  label.btn:focus-within { outline: 2px solid var(--accent); outline-offset: 2px; }
  .file { position: absolute; width: 1px; height: 1px; opacity: 0; pointer-events: none; }
  .swatches, .chips { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
  .swatch { width: 40px; height: 28px; border-radius: 8px; border: 2px solid transparent; padding: 0; cursor: pointer; }
  .swatch.round { width: 28px; border-radius: 50%; }
  .swatch[aria-checked='true'] { border-color: var(--text); box-shadow: 0 0 0 2px var(--bg-1) inset; }
  .chip { height: 30px; padding: 0 12px; border: 1px solid var(--bg-4); border-radius: 15px; background: transparent; color: var(--text-2); cursor: pointer; }
  .chip[aria-checked='true'] { background: var(--bg-3); border-color: var(--accent); color: var(--text); }
  input[type='color'] { width: 44px; height: 30px; padding: 0 2px; border: 1px solid var(--bg-4); border-radius: 8px; background: transparent; cursor: pointer; }
  .text { height: 36px; padding: 0 12px; border: 1px solid var(--bg-4); border-radius: 10px; background: var(--bg-0); color: var(--text); }
  .settings-field :global(.bio) { min-height: 72px; max-height: 160px; overflow-y: auto; padding: 8px 12px; border: 1px solid var(--bg-4); border-radius: 10px; background: var(--bg-0); white-space: pre-wrap; }
  .counter { align-self: flex-end; }
  .counter.over { color: var(--danger); font-weight: 600; }
  .line { display: flex; gap: 12px; align-items: flex-start; padding: 12px; border-radius: 12px; background: var(--bg-2); }
  .said { min-width: 0; display: flex; flex-direction: column; font-weight: 600; }
  .said p { margin: 2px 0 0; font-weight: 400; color: var(--text); }
  .footer { position: sticky; bottom: -28px; display: flex; align-items: center; justify-content: flex-end; flex-wrap: wrap; gap: 8px; padding: 12px 0; background: var(--bg-1); border-top: 1px solid var(--line); }
  .footer .error { flex-basis: 100%; }
  .state { margin-right: auto; color: var(--warn); font-weight: 600; font-size: 13px; }
  .state .ok { color: var(--ok); }
  .muted { color: var(--text-3); font-size: 13px; }
  .error { margin: 0; color: var(--danger); font-size: 13px; }
</style>
