<script lang="ts">
  // Settings → Profile (spec §4): settings on the left, a live preview on the right. Everything,
  // uploads included, is staged in the draft until Save sends it in one request; the store isn't
  // touched before then, so leaving without saving leaves you exactly as you were.
  import { flushSync, onDestroy, untrack } from 'svelte'
  import Icon from './Icon.svelte'
  import ProfileCard from './ProfileCard.svelte'
  import Avatar from './Avatar.svelte'
  import Name from './Name.svelte'
  import CropDialog from './CropDialog.svelte'
  import EmojiBox from './EmojiBox.svelte'
  import Segmented from './Segmented.svelte'
  import { app, loadProfile } from '../lib/store.svelte'
  import { setProfile } from '../lib/state'
  import { api, errorText } from '../lib/tauri'
  import { BIO_LINE_BREAKS_MAX, BIO_MAX, charCount, draftFrom, editsSince, isDirty, isZalgo, nameProblem, normalBio, previewCard, previewUser, PRONOUNS_MAX, toRequest, type Draft } from '../lib/profileDraft'
  import { radioKeys } from '../lib/actions'
  import { presetFor, THEME_PRESETS } from '../lib/cardTheme'
  import { NAME_FONTS } from '../lib/nameFonts'
  import { lightenForName, NAME_GRADIENTS, NAME_SOLIDS, type NamePreset } from '../lib/nameStyle'
  import type { CropRect } from '../lib/crop'
  import type { Profile } from '../lib/protocol/Profile'
  import { perFrame } from '../lib/perFrame'
  import Loading from './Loading.svelte'

  let { dirty = $bindable(false), settling = $bindable(null) }: {
    /** Unsaved changes, an upload still in flight included: leaving would lose them. */
    dirty?: boolean
    /** A Save in flight: whoever closes the editor waits for it (it may still fail and keep the draft). */
    settling?: Promise<void> | null
  } = $props()

  const uid = $props.id()
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
  const nameError = $derived(draft ? nameProblem(draft.displayName) : null)
  const invalid = $derived(bioOver || bioBreaksOver || pronounsOver || !!nameError)
  // Accounts from before display names have their username as one: nudge once (#47), until dismissed.
  const NUDGE_KEY = (id: string) => `pulse.nameNudgeDismissed.${id}`
  let nudgeGone = $state(false)
  const nudge = $derived.by(() => {
    const u = profile?.user
    if (!me || !u || nudgeGone || (u.display_name && u.display_name !== u.username)) return false
    try {
      return localStorage.getItem(NUDGE_KEY(me.id)) === null
    } catch {
      return true
    }
  })
  function dismissNudge() {
    nudgeGone = true
    try {
      if (me) localStorage.setItem(NUDGE_KEY(me.id), '1')
    } catch {
      // no storage: it stays dismissed until Settings closes
    }
  }

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
  // A native colour picker fires `input` many times a frame while you drag; each wrote the draft and redrew the preview
  // card (#156 follow-up). One write per frame, the last value; `change` (the picker closing) writes at once.
  const pickers = {
    top: perFrame((v: string) => draft && (draft.themeTop = v)),
    bottom: perFrame((v: string) => draft && (draft.themeBottom = v)),
    colours: [0, 1].map((i) => perFrame((v: string) => draft && i < draft.colors.length && (draft.colors[i] = v))),
  }
  const allPickers = [pickers.top, pickers.bottom, ...pickers.colours]
  onDestroy(() => {
    gone = true
    clearTimeout(savedTimer)
    for (const p of allPickers) p.cancel()
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
    pickers.top.cancel()
    pickers.bottom.cancel()
    customTheme = false
    draft.themeTop = top
    draft.themeBottom = bottom
  }

  function setColours(colors: string[], custom = false) {
    if (!draft) return
    for (const p of pickers.colours) p.cancel()
    customColour = custom
    draft.colors = [...colors]
  }
  // Custom (#165) opens the colour picker at once, but not when the arrow keys move onto it (radioKeys clicks the
  // radio it lands on): a native picker popping up mid-arrowing would trap the keys.
  let arrowing = false
  let colourField = $state<HTMLElement>()
  function colourKeys(e: KeyboardEvent & { currentTarget: EventTarget & HTMLElement }) {
    arrowing = true
    try {
      radioKeys(e)
    } finally {
      arrowing = false
    }
  }
  function pickCustomColour() {
    if (!draft) return
    setColours(draft.colors.length ? draft.colors : ['#ffffff'], true)
    if (arrowing) return
    flushSync()
    colourField?.querySelector<HTMLInputElement>('input[type=color]')?.click()
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
    for (const p of allPickers) p.cancel()
    gen.avatar++
    gen.banner++
    uploading = { avatar: false, banner: false }
    take(profile)
    uploadError = { avatar: null, banner: null }
    saveError = null
    showBio(saved!.bio)
  }

  function save() {
    for (const p of allPickers) p.flush()
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
      <label class="btn btn-secondary upload" class:disabled={uploading[kind]}>Upload
        <input class="file" type="file" accept={ACCEPT} aria-label="Upload {kind}" disabled={uploading[kind]} onchange={(e) => pickFile(kind, e)} />
      </label>
      <!-- during an upload, Remove cancels it -->
      <button type="button" class="btn btn-secondary" disabled={!hash && !uploading[kind]} onclick={() => remove(kind)}>Remove</button>
      {#if uploading[kind]}<span class="muted" role="status">Uploading…</span>{/if}
    </div>
    {#if uploadError[kind]}<p class="error" role="alert">{uploadError[kind]}</p>{/if}
  </div>
{/snippet}

<section class="settings-pane">
  <h2>Profile</h2>
  {#if loadError}
    <p class="error" role="alert">{loadError} <button type="button" class="btn btn-secondary retry" onclick={() => me && load(me.id)}>Retry</button></p>
  {:else if !draft || !me || !pu || !pc || !profile}
    <Loading />
  {:else}
    <div class="layout">
      <div class="settings">
        {#if nudge}
          <div class="nudge" role="note">
            <span><strong>Pick how your name looks.</strong> Capitals, spaces and emoji are all fine; your username stays for signing in.</span>
            <button type="button" class="btn btn-secondary" onclick={dismissNudge}>Dismiss</button>
          </div>
        {/if}
        <div class="settings-field name-field">
          <span class="settings-label">Display name</span>
          <!-- counted in characters, as the server counts -->
          <input class="input text" type="text" aria-label="Display name" aria-invalid={!!nameError} bind:value={draft.displayName} />
          <!-- the Zalgo easter egg (#209): a soft nudge, not an error; it still saves -->
          {#if nameError}<p class="error" role="alert">{nameError}</p>{:else if isZalgo(draft.displayName)}<small class="zalgo" role="status">Zalgo? Really?</small>{:else}<small>How your name shows everywhere. Not unique: your @{me.username} tells you apart.</small>{/if}
        </div>

        {@render picture('avatar', 'Avatar', draft.avatarHash)}
        {@render picture('banner', 'Banner', draft.bannerHash)}

        <div class="settings-field theme">
          <span class="settings-label">Card theme</span>
          <div class="swatches" role="radiogroup" aria-label="Card theme" tabindex="-1" onkeydown={radioKeys}>
            {#each THEME_PRESETS as p (p.key)}
              <button type="button" role="radio" class="swatch" aria-label={p.label} aria-checked={themeKey === p.key} tabindex={themeKey === p.key ? 0 : -1}
                style:background="linear-gradient({p.top}, {p.bottom})" onclick={() => setTheme(p.top, p.bottom)}></button>
            {/each}
            <!-- Custom shows the colours it holds (#141: next to the presets, pale ones looked like nothing was picked) -->
            <button type="button" role="radio" class="chip toggle-chip custom-chip" aria-checked={themeKey === null} tabindex={themeKey === null ? 0 : -1} onclick={() => (customTheme = true)}>
              {#if themeKey === null}<span class="mini" style:background="linear-gradient({draft.themeTop}, {draft.themeBottom})" aria-hidden="true"></span>{/if}Custom</button>
          </div>
          {#if themeKey === null}
            <div class="row">
              <label class="pick">
                <input type="color" aria-label="Top colour" value={draft.themeTop} oninput={(e) => pickers.top.push(e.currentTarget.value)} onchange={() => pickers.top.flush()} />
                <span>Top <code>{draft.themeTop}</code></span>
              </label>
              <label class="pick">
                <input type="color" aria-label="Bottom colour" value={draft.themeBottom} oninput={(e) => pickers.bottom.push(e.currentTarget.value)} onchange={() => pickers.bottom.flush()} />
                <span>Bottom <code>{draft.themeBottom}</code></span>
              </label>
            </div>
          {/if}
        </div>

        <div class="settings-field">
          <label class="toggle-row">
            <input type="checkbox" role="switch" class="switch" aria-describedby="{uid}-glow" bind:checked={draft.glow} />
            <span>Card glow</span>
          </label>
          <small id="{uid}-glow">A soft glow in your card's colours around it. Everyone sees your card as you set it.</small>
        </div>

        <div class="settings-field">
          <span class="settings-label">Name font</span>
          <div class="chips" role="radiogroup" aria-label="Name font" tabindex="-1" onkeydown={radioKeys}>
            <button type="button" role="radio" class="chip toggle-chip" aria-checked={draft.font === null} tabindex={fontKnown ? (draft.font === null ? 0 : -1) : 0} onclick={() => (draft!.font = null)}>Default</button>
            {#each NAME_FONTS as f (f.key)}
              <button type="button" role="radio" class="chip toggle-chip" aria-checked={draft.font === f.key} tabindex={draft.font === f.key ? 0 : -1}
                style="font-family: {f.family}; font-weight: {f.weight}; font-size: calc(13px * {f.scale})" onclick={() => (draft!.font = f.key)}>{f.label}</button>
            {/each}
          </div>
        </div>

        <div class="settings-field colour" bind:this={colourField}>
          <span class="settings-label">Name colour</span>
          <div class="swatches" role="radiogroup" aria-label="Name colour" tabindex="-1" onkeydown={colourKeys}>
            <!-- no colour: a struck-through swatch, sized like the presets beside it -->
            <button type="button" role="radio" class="swatch round none" aria-label="Default" data-tip="Default colour" aria-checked={colourKey === 'default'} tabindex={colourKey === 'default' ? 0 : -1} onclick={() => setColours([])}></button>
            {#each NAME_SOLIDS as p (p.key)}
              <button type="button" role="radio" class="swatch round" aria-label={p.label} aria-checked={colourKey === `solid:${p.key}`} tabindex={colourKey === `solid:${p.key}` ? 0 : -1}
                style:background={p.colors[0]} onclick={() => setColours(p.colors)}></button>
            {/each}
            {#each NAME_GRADIENTS as p (p.key)}
              <button type="button" role="radio" class="swatch round" aria-label="{p.label} gradient" aria-checked={colourKey === `gradient:${p.key}`} tabindex={colourKey === `gradient:${p.key}` ? 0 : -1}
                style:background="linear-gradient(135deg, {p.colors[0]}, {p.colors[1]})" onclick={() => setColours(p.colors)}></button>
            {/each}
            <button type="button" role="radio" class="swatch round pick" aria-label="Custom" data-tip="Custom colour" aria-checked={colourKey === 'custom'} tabindex={colourKey === 'custom' ? 0 : -1} onclick={pickCustomColour}><Icon name="eyedropper" size={14} /></button>
          </div>
          {#if colourKey === 'custom'}
            <Segmented label="Custom colour" value={draft.colors.length >= 2 ? 'gradient' : 'solid'} onchange={setCustomCount}
              options={[{ value: 'solid', label: 'Solid' }, { value: 'gradient', label: 'Gradient' }]} />
            <div class="row">
              {#each draft.colors as c, i}
                <input type="color" aria-label={draft.colors.length >= 2 ? (i === 0 ? 'First colour' : 'Second colour') : 'Colour'} value={c}
                  oninput={(e) => pickers.colours[i].push(e.currentTarget.value)} onchange={() => pickers.colours[i].flush()} />
              {/each}
            </div>
            {#if lightened}<small>Lightened so it reads on dark backgrounds.</small>{/if}
          {/if}
        </div>

        <div class="settings-field">
          <span class="settings-label">Pronouns</span>
          <!-- counted in characters, as the server counts (maxlength counts UTF-16 units) -->
          <input class="input text" type="text" aria-label="Pronouns" aria-invalid={pronounsOver} bind:value={draft.pronouns} />
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
        <ProfileCard user={pu} card={pc} badges={profile.badges} showcase={profile.showcase ?? []} stats={profile.stats} serverId={null} preview />
        <div class="line">
          <Avatar id={me.id} name={pu.display_name} user={pu} size={36} />
          <div class="said">
            <Name userId={me.id} serverId={null} user={pu} text={pu.display_name} interactive={false} />
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
      <button type="button" class="btn btn-lg btn-secondary" disabled={!(changed || busy) || saving} onclick={reset}>Reset</button>
      <button type="button" class="btn btn-lg btn-primary" disabled={!changed || saving || invalid || busy} onclick={save}>Save</button>
    </div>
  {/if}
</section>

{#if cropping}
  {#key cropping}
    <CropDialog file={cropping.file} kind={cropping.kind} onApply={(c) => void upload(c)} onCancel={() => (cropping = null)} />
  {/key}
{/if}

<style>
  /* the preview column: the quick card's width (ProfileCard, #156) */
  .layout { display: grid; grid-template-columns: minmax(0, 1fr) 336px; gap: 24px; align-items: start; }
  .settings { display: flex; flex-direction: column; gap: 20px; min-width: 0; }
  .preview { position: sticky; top: 0; display: flex; flex-direction: column; gap: 16px; }
  @media (max-width: 860px) {
    .layout { grid-template-columns: minmax(0, 1fr); }
    .preview { position: static; order: -1; }
  }
  .row { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  /* the file input sits under its label, which is the button people see */
  .upload { position: relative; cursor: pointer; }
  .upload:focus-within { outline: 2px solid var(--focus-ring); outline-offset: 2px; }
  .upload.disabled { opacity: .5; cursor: default; }
  .file { position: absolute; width: 1px; height: 1px; opacity: 0; pointer-events: none; }
  .swatches, .chips { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
  /* a hairline inside every swatch: a white or pale one still reads as a swatch on a light theme (#141) */
  .swatch { width: 40px; height: 28px; border-radius: var(--radius-sm); border: 2px solid transparent; padding: 0; cursor: pointer; outline: 1px solid var(--line-strong); outline-offset: -1px; }
  .swatch[aria-checked='true'] { outline: none; }
  .swatch:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 2px; }
  .swatch.round { width: 28px; border-radius: 50%; }
  .swatch[aria-checked='true'] { border-color: var(--text); box-shadow: 0 0 0 2px var(--ring) inset; }
  /* Default (no colour) and Custom: swatch-sized, drawn in the theme's own greys */
  .swatch.none { border-color: var(--bg-4); background: linear-gradient(135deg, transparent calc(50% - 1px), var(--text-3) calc(50% - 1px) calc(50% + 1px), transparent calc(50% + 1px)); }
  .swatch.pick { border-color: var(--bg-4); background: var(--bg-3); color: var(--text-2); display: grid; place-items: center; }
  .swatch.pick:hover { color: var(--text); }
  .swatch.none[aria-checked='true'], .swatch.pick[aria-checked='true'] { border-color: var(--text); }
  /* the shared .toggle-chip (app.css); a font's name keeps the body size, so the sample reads */
  .chip { font-size: 14px; font-weight: 400; }
  /* The colour wells (#141): the whole well is the colour, with a hairline edge that shows round a white or pale one on
     any theme, and the hex beside it. */
  input[type='color'] { width: 44px; height: 32px; padding: 0; border: 1px solid var(--line-strong); border-radius: var(--radius-sm); background: transparent; cursor: pointer; overflow: hidden; }
  input[type='color']::-webkit-color-swatch-wrapper { padding: 0; }
  input[type='color']::-webkit-color-swatch { border: 0; border-radius: 0; }
  input[type='color']::-moz-color-swatch { border: 0; border-radius: 0; }
  .pick { display: inline-flex; align-items: center; gap: 8px; font-size: 13px; color: var(--text-2); cursor: pointer; }
  .pick code { font-size: 12px; color: var(--text-3); }
  .custom-chip { display: inline-flex; align-items: center; gap: 6px; }
  .mini { width: 14px; height: 14px; border-radius: 50%; box-shadow: inset 0 0 0 1px var(--line-strong); }
  .toggle-row { display: flex; align-items: center; gap: 10px; font-weight: 600; cursor: pointer; }
  /* the shared field ring (EmojiBox turns the box's own outline off) */
  .settings-field :global(.bio:focus-visible) { outline: 2px solid var(--focus-ring); outline-offset: 2px; }
  .settings-field :global(.bio) { min-height: 72px; max-height: 160px; overflow-y: auto; padding: 8px 12px; border: 1px solid var(--bg-4); border-radius: var(--radius-md); background: var(--bg-1); white-space: pre-wrap; }
  .counter { align-self: flex-end; }
  .nudge { display: flex; align-items: center; gap: 12px; padding: 10px 12px; border-radius: var(--radius-md); background: var(--accent-soft); color: var(--text); font-size: 13px; }
  .nudge span { flex: 1; }
  .counter.over { color: var(--danger-text); font-weight: 600; }
  .line { display: flex; gap: 12px; align-items: flex-start; padding: 12px; border-radius: var(--radius-lg); background: var(--bg-2); }
  .said { min-width: 0; display: flex; flex-direction: column; font-weight: 600; }
  .said p { margin: 2px 0 0; font-weight: 400; color: var(--text); }
  /* flush with the bottom of Settings' scrolling content, whatever its padding */
  .footer { position: sticky; bottom: calc(-1 * var(--settings-pad-y, 28px)); display: flex; align-items: center; justify-content: flex-end; flex-wrap: wrap; gap: 8px; padding: 12px 0; background: var(--bg-1); border-top: 1px solid var(--line); }
  .footer .error { flex-basis: 100%; }
  .state { margin-right: auto; color: var(--warn-text); font-weight: 600; font-size: 13px; }
  .state .ok { color: var(--ok-text); }
  .muted { color: var(--text-3); font-size: 13px; }
  .error { margin: 0; color: var(--danger-text); font-size: 13px; }
  .retry { height: 28px; margin-left: 8px; }
</style>
