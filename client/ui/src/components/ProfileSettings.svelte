<script lang="ts">
  // Settings → Profile (spec §4): settings on the left, a live preview on the right. Everything,
  // uploads included, is staged in the draft until Save sends it in one request; the store isn't
  // touched before then, so leaving without saving leaves you exactly as you were.
  import { onDestroy, untrack } from 'svelte'
  import ProfileCard from './ProfileCard.svelte'
  import Avatar from './Avatar.svelte'
  import Name from './Name.svelte'
  import CropDialog from './CropDialog.svelte'
  import EmojiBox from './EmojiBox.svelte'
  import Segmented from './Segmented.svelte'
  import { app, loadProfile } from '../lib/store.svelte'
  import { setProfile } from '../lib/state'
  import { api, errorText } from '../lib/tauri'
  import { BIO_LINE_BREAKS_MAX, BIO_MAX, charCount, draftFrom, editsSince, isDirty, normalBio, previewCard, previewUser, PRONOUNS_MAX, toRequest, type Draft } from '../lib/profileDraft'
  import { radioKeys } from '../lib/actions'
  import { presetFor, THEME_PRESETS } from '../lib/cardTheme'
  import { NAME_FONTS } from '../lib/nameFonts'
  import { lightenForName, NAME_GRADIENTS, NAME_SOLIDS, type NamePreset } from '../lib/nameStyle'
  import type { CropRect } from '../lib/crop'
  import type { Profile } from '../lib/protocol/Profile'

  let { dirty = $bindable(false), settling = $bindable(null) }: {
    /** Unsaved changes, an upload still in flight included: leaving would lose them. */
    dirty?: boolean
    /** A Save in flight: whoever closes the editor waits for it (it may still fail and keep the draft). */
    settling?: Promise<void> | null
  } = $props()

  type Kind = 'avatar' | 'banner'
  const ACCEPT = 'image/png,image/jpeg,image/gif,image/webp'
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
  /** Bumped per kind by each upload, Remove and Reset: an upload that lands after a newer one of these is dropped. */
  const gen: Record<Kind, number> = { avatar: 0, banner: 0 }

  const me = $derived(app.state.me)
  const pu = $derived(me && draft ? previewUser(me, draft) : null)
  const pc = $derived(draft ? previewCard(draft) : null)
  const changed = $derived(!!draft && !!saved && isDirty(draft, saved))
  const busy = $derived(uploading.avatar || uploading.banner)
  $effect(() => {
    dirty = changed || busy
  })

  const fontKnown = $derived(!!draft && (draft.font === null || NAME_FONTS.some((f) => f.key === draft!.font)))
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
  // Counted as the server counts: characters (code points) of the bio as it will be stored.
  const bio = $derived(draft ? normalBio(draft.bio) : '')
  const bioLength = $derived(charCount(bio))
  const bioOver = $derived(bioLength > BIO_MAX)
  const bioBreaksOver = $derived((bio.match(/\n/g)?.length ?? 0) > BIO_LINE_BREAKS_MAX)
  const pronounsOver = $derived(!!draft && charCount(draft.pronouns.trim()) > PRONOUNS_MAX)
  const invalid = $derived(bioOver || bioBreaksOver || pronounsOver)

  function take(p: Profile) {
    profile = p
    saved = draftFrom(p)
    draft = draftFrom(p)
    customTheme = false
    customColour = false
  }

  function load(id: string) {
    loadError = null
    loadProfile(id).then(
      (p) => !gone && !profile && p.user.id === app.state.me?.id && take(p),
      (e) => !gone && !profile && (loadError = errorText(e)),
    )
  }
  // Loads once you are known (you may not be yet when the editor opens).
  let loadedFor: string | null = null
  $effect(() => {
    const id = me?.id
    if (!id || id === loadedFor) return
    loadedFor = id
    untrack(() => load(id))
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
    const g = ++gen[kind]
    uploadError[kind] = null
    uploading[kind] = true
    try {
      const bytes = new Uint8Array(await file.arrayBuffer())
      const info = await (kind === 'avatar' ? api.uploadAvatar(bytes, crop) : api.uploadBanner(bytes, crop))
      if (!draft || gone || gen[kind] !== g) return // Reset or Remove since: dropped
      if (kind === 'avatar') {
        draft.avatarHash = info.hash
        draft.avatarStill = info.still_hash
      } else {
        draft.bannerHash = info.hash
        draft.bannerStill = info.still_hash
      }
    } catch (e) {
      if (!gone && gen[kind] === g) uploadError[kind] = errorText(e)
    } finally {
      if (!gone && gen[kind] === g) uploading[kind] = false
    }
  }

  /** Clears the picture, and drops an upload of it still in flight. */
  function remove(kind: Kind) {
    if (!draft) return
    gen[kind]++
    uploading[kind] = false
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
    gen.avatar++
    gen.banner++
    uploading = { avatar: false, banner: false }
    take(profile)
    uploadError = { avatar: null, banner: null }
    saveError = null
    showBio(saved!.bio)
  }

  function save() {
    if (!draft || saving || !changed || invalid || busy) return
    const p = send($state.snapshot(draft))
    settling = p
    void p.finally(() => {
      if (!gone && settling === p) settling = null
    })
  }

  /** Sends `sent`; edits made while it is on its way are kept on top of the answer. */
  async function send(sent: Draft) {
    saving = true
    saveError = null
    try {
      const answer = await api.updateProfile(toRequest(sent))
      // The cache gets the answer even if the editor has gone meanwhile: the save went through.
      app.state = setProfile(app.state, answer)
      if (gone) return
      const edits = draft ? editsSince(sent, $state.snapshot(draft)) : {}
      const keepTheme = customTheme && ('themeTop' in edits || 'themeBottom' in edits)
      const keepColour = customColour && 'colors' in edits
      take(answer)
      Object.assign(draft!, edits)
      customTheme = keepTheme
      customColour = keepColour
      if (!('bio' in edits)) showBio(answer.card.bio ?? '')
      justSaved = true
      clearTimeout(savedTimer)
      savedTimer = setTimeout(() => (justSaved = false), SAVED_MS)
    } catch (e) {
      if (!gone) saveError = errorText(e)
    } finally {
      if (!gone) saving = false
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
      <!-- during an upload, Remove cancels it -->
      <button type="button" class="btn" disabled={!hash && !uploading[kind]} onclick={() => remove(kind)}>Remove</button>
      {#if uploading[kind]}<span class="muted" role="status">Uploading…</span>{/if}
    </div>
    {#if uploadError[kind]}<p class="error" role="alert">{uploadError[kind]}</p>{/if}
  </div>
{/snippet}

<section class="settings-pane">
  <h2>Profile</h2>
  {#if loadError}
    <p class="error" role="alert">{loadError} <button type="button" class="btn retry" onclick={() => me && load(me.id)}>Retry</button></p>
  {:else if !draft || !me || !pu || !pc || !profile}
    <p class="muted">Loading…</p>
  {:else}
    <div class="layout">
      <div class="settings">
        {@render picture('avatar', 'Avatar', draft.avatarHash)}
        {@render picture('banner', 'Banner', draft.bannerHash)}

        <div class="settings-field theme">
          <span class="settings-label">Card theme</span>
          <div class="swatches" role="radiogroup" aria-label="Card theme" tabindex="-1" onkeydown={radioKeys}>
            {#each THEME_PRESETS as p (p.key)}
              <button type="button" role="radio" class="swatch" aria-label={p.label} title={p.label} aria-checked={themeKey === p.key} tabindex={themeKey === p.key ? 0 : -1}
                style:background="linear-gradient({p.top}, {p.bottom})" onclick={() => setTheme(p.top, p.bottom)}></button>
            {/each}
            <button type="button" role="radio" class="chip" aria-checked={themeKey === null} tabindex={themeKey === null ? 0 : -1} onclick={() => (customTheme = true)}>Custom</button>
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
          <div class="chips" role="radiogroup" aria-label="Name font" tabindex="-1" onkeydown={radioKeys}>
            <button type="button" role="radio" class="chip" aria-checked={draft.font === null} tabindex={fontKnown ? (draft.font === null ? 0 : -1) : 0} onclick={() => (draft!.font = null)}>Default</button>
            {#each NAME_FONTS as f (f.key)}
              <button type="button" role="radio" class="chip" aria-checked={draft.font === f.key} tabindex={draft.font === f.key ? 0 : -1}
                style="font-family: {f.family}; font-weight: {f.weight}; font-size: calc(13px * {f.scale})" onclick={() => (draft!.font = f.key)}>{f.label}</button>
            {/each}
          </div>
        </div>

        <div class="settings-field colour">
          <span class="settings-label">Name colour</span>
          <div class="swatches" role="radiogroup" aria-label="Name colour" tabindex="-1" onkeydown={radioKeys}>
            <button type="button" role="radio" class="chip" aria-checked={colourKey === 'default'} tabindex={colourKey === 'default' ? 0 : -1} onclick={() => setColours([])}>Default</button>
            {#each NAME_SOLIDS as p (p.key)}
              <button type="button" role="radio" class="swatch round" aria-label={p.label} title={p.label} aria-checked={colourKey === `solid:${p.key}`} tabindex={colourKey === `solid:${p.key}` ? 0 : -1}
                style:background={p.colors[0]} onclick={() => setColours(p.colors)}></button>
            {/each}
            {#each NAME_GRADIENTS as p (p.key)}
              <button type="button" role="radio" class="swatch round" aria-label="{p.label} gradient" title="{p.label} gradient" aria-checked={colourKey === `gradient:${p.key}`} tabindex={colourKey === `gradient:${p.key}` ? 0 : -1}
                style:background="linear-gradient(135deg, {p.colors[0]}, {p.colors[1]})" onclick={() => setColours(p.colors)}></button>
            {/each}
            <button type="button" role="radio" class="chip" aria-checked={colourKey === 'custom'} tabindex={colourKey === 'custom' ? 0 : -1} onclick={pickCustomColour}>Custom</button>
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
          <!-- counted in characters, as the server counts (maxlength counts UTF-16 units) -->
          <input class="text" type="text" aria-label="Pronouns" aria-invalid={pronounsOver} bind:value={draft.pronouns} />
          {#if pronounsOver}<p class="error" role="alert">Pronouns can be up to 40 characters.</p>{/if}
        </div>

        <div class="settings-field">
          <span class="settings-label">Bio</span>
          <EmojiBox bind:this={bioBox} bind:text={draft.bio} label="Bio" class="bio" />
          <small class="counter" class:over={bioOver}>{bioLength}/{BIO_MAX}</small>
          {#if bioBreaksOver}<p class="error" role="alert">Your bio can have at most 4 line breaks.</p>{/if}
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
      <button type="button" class="btn" disabled={!(changed || busy) || saving} onclick={reset}>Reset</button>
      <button type="button" class="btn primary" disabled={!changed || saving || invalid || busy} onclick={save}>Save</button>
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
  .btn:not(.primary):not(.disabled):hover:not(:disabled) { background: var(--bg-3); }
  .btn.primary:not(.disabled):hover:not(:disabled) { filter: brightness(1.08); }
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
  /* flush with the bottom of Settings' scrolling content, whatever its padding */
  .footer { position: sticky; bottom: calc(-1 * var(--settings-pad-y, 28px)); display: flex; align-items: center; justify-content: flex-end; flex-wrap: wrap; gap: 8px; padding: 12px 0; background: var(--bg-1); border-top: 1px solid var(--line); }
  .footer .error { flex-basis: 100%; }
  .state { margin-right: auto; color: var(--warn); font-weight: 600; font-size: 13px; }
  .state .ok { color: var(--ok); }
  .muted { color: var(--text-3); font-size: 13px; }
  .error { margin: 0; color: var(--danger); font-size: 13px; }
  .retry { height: 28px; margin-left: 8px; }
</style>
