<script lang="ts">
  // Settings → Profile (spec §4; redesigned in #467): five tabs (About you, Card, Name style, Speaking ring, Badges) on the left, the
  // live preview always beside them (on top in a narrow window). Hovering the preview's avatar or banner offers to
  // change it. Everything, uploads and the badge order included, is staged until Save sends it; the store isn't
  // touched before then, so leaving without saving leaves you exactly as you were. Explanations are tooltips.
  import { flushSync, onDestroy, untrack } from 'svelte'
  import Icon from './Icon.svelte'
  import ProfileCard from './ProfileCard.svelte'
  import Avatar from './Avatar.svelte'
  import Name from './Name.svelte'
  import CropDialog from './CropDialog.svelte'
  import EmojiBox from './EmojiBox.svelte'
  import Segmented from './Segmented.svelte'
  import BadgeIcon from './BadgeIcon.svelte'
  import { app, loadProfile } from '../lib/store.svelte'
  import { errorText } from '../lib/tauri'
  import { ProfileEditorState } from '../lib/profileEditor.svelte'
  import { TYPING_SUGGESTIONS, typingTextProblem } from '../lib/typingLine'
  import { BIO_LINE_BREAKS_MAX, BIO_MAX, charCount, isZalgo, nameProblem, normalBio, previewCard, previewUser, PRONOUNS_MAX } from '../lib/profileDraft'
  import { clickOutside, radioKeys } from '../lib/actions'
  import { cardVars, presetFor, THEME_PRESETS } from '../lib/cardTheme'
  import { SHOWCASE_MAX, badgeStore, fallbackName } from '../lib/badges.svelte'
  import { railDrag } from '../lib/railDrag'
  import type { Badge } from '../lib/protocol/Badge'
  import { NAME_FONTS } from '../lib/nameFonts'
  import { NAME_EFFECTS } from '../lib/nameEffects'
  import { lightenForName, NAME_GRADIENTS, NAME_SOLIDS, type NamePreset } from '../lib/nameStyle'
  import type { CropRect } from '../lib/crop'
    import { perFrame } from '../lib/perFrame'
  import Loading from './Loading.svelte'
  import SpeakingRing from './SpeakingRing.svelte'
  import { motion, observe } from '../lib/animate.svelte'
  import { windowFocus } from '../lib/windowFocus.svelte'
  import { envelopeStep, isReactive, PREVIEW_D, RING_COLORS, RING_STYLES, ringState, speakCss, type RingColor } from '../lib/speakingRing'
  import { nameBg } from '../lib/theme.svelte'

  let { dirty = $bindable(false), settling = $bindable(null) }: {
    /** Unsaved changes, an upload still in flight included: leaving would lose them. */
    dirty?: boolean
    /** A Save in flight: whoever closes the editor waits for it (it may still fail and keep the draft). */
    settling?: Promise<void> | null
  } = $props()

  const uid = $props.id()
  type Kind = 'avatar' | 'banner'
  type Tab = 'about' | 'card' | 'name' | 'ring' | 'badges'
  const TABS: { key: Tab; label: string }[] = [
    { key: 'about', label: 'About you' }, { key: 'card', label: 'Card' }, { key: 'name', label: 'Name style' }, { key: 'ring', label: 'Speaking ring' }, { key: 'badges', label: 'Badges' },
  ]
  let tab = $state<Tab>('about')
  /** The bio counter shows from here on (#467: only near the limit). */
  const BIO_NEAR = 150
  const ACCEPT = 'image/png,image/jpeg,image/gif,image/webp'
  const SAVED_MS = 2000

  // The working copy is the editor's shared state (lib/profileEditor.svelte.ts); this pane is one view of it.
  let st = $state.raw<ProfileEditorState | null>(null)
  const profile = $derived(st?.profile ?? null)
  const draft = $derived(st?.draft ?? null)
  let loadError = $state<string | null>(null)
  // "Custom" picked while the colours still match a preset: show the pickers anyway.
  let customTheme = $state(false)
  let customColour = $state(false)
  let cropping = $state<{ file: File; kind: Kind } | null>(null)
  const uploading = $derived(st?.uploading ?? { avatar: false, banner: false })
  const uploadError = $derived(st?.uploadErrors ?? { avatar: null, banner: null })
  const saving = $derived(st?.saving ?? false)
  const saveError = $derived(st?.error || null)
  let justSaved = $state(false)
  let savedTimer: ReturnType<typeof setTimeout> | undefined
  let bioBox = $state<EmojiBox>()
  let gone = false

  // The card's badges (#467's Badges tab): up to SHOWCASE_MAX held badges in order, or `auto` (the rarest five, which
  // follows later unlocks). Saved with the rest.
  const showcaseAuto = $derived(st?.showcase === 'auto')
  const held = $derived(new Map<string, Badge>((profile?.badges ?? []).map((b) => [b.key, b])))
  const shownBadges = $derived(st?.shown ?? [])
  const wantsCatalogue = $derived(held.size > 0)
  $effect(() => {
    if (wantsCatalogue) untrack(() => void badgeStore.load())
  })

  const me = $derived(app.state.me)
  const pu = $derived(me && draft ? previewUser(me, draft) : null)
  const pc = $derived(draft ? previewCard(draft) : null)
  const changed = $derived(st?.dirty ?? false)
  const busy = $derived(st?.busy ?? false)
  $effect(() => {
    dirty = changed || busy
  })

  // the effect chip under the pointer or keyboard plays (#468); the rest sit still
  let effectHot = $state<string | null>(null)
  const effectKnown = $derived(!!draft && (draft.effect === null || NAME_EFFECTS.some((e) => e.key === draft!.effect)))
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
  const typingError = $derived(draft ? typingTextProblem(draft.typingText) : null)
  /** Your name as the draft has it, for "<name> is …" (#652). */
  const typingName = $derived(draft && me ? draft.displayName.trim() || me.username : '')
  const invalid = $derived(st?.invalid ?? false)
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

  // --- the speaking ring (#651 §7.1): a style, a colour, and a live preview of your draft ---
  const RING_GROUPS = [
    { key: 'looping', label: 'Still or looping', styles: RING_STYLES.filter((r) => !r.reactive) },
    { key: 'voice', label: 'Follows your voice', styles: RING_STYLES.filter((r) => r.reactive) },
  ]
  /** A swatch's colour, as the ring would draw it on this theme (the light themes' White is ink). */
  const swatchCss = (c: RingColor) => speakCss(pu, c, nameBg('side'))
  const profileTip = $derived(draft?.colors.length ? 'Your name colour, from Name style' : "Your name colour, from Name style. You haven't set one, so it's green.")
  // The preview's fake speech (a reactive style only): --lvl written straight onto the stage every 70 ms, as the call
  // tiles get theirs, and only while the tab is shown and motion is allowed.
  let ringStage = $state<HTMLElement>()
  /** The preview as SpeakingRing draws it: still under reduced motion or while you're live, as call tiles are. */
  const previewRing = $derived(draft ? ringState({ lit: true, place: 'preview', style: draft.ringStyle, reducedMotion: motion.reducedMotion, streaming: motion.streaming, animating: true }) : 'off')
  const previewCaption = $derived(previewRing !== 'still' ? 'Preview · talking' : motion.reducedMotion ? 'Preview · still (reduced motion is on)' : "Preview · still (you're live)")
  // on screen (a hidden tab or a scrolled-away preview isn't), and the window focused: what animated emoji follow
  let stageSeen = $state(false)
  $effect(() => {
    const el = ringStage
    if (!el) return
    return observe(el, (v) => (stageSeen = v))
  })
  const envelopeOn = $derived(tab === 'ring' && previewRing === 'on' && stageSeen && windowFocus.focused && !!draft && isReactive(draft.ringStyle))
  $effect(() => {
    const el = ringStage
    if (!el || !envelopeOn) return
    let s = { lvl: 0, pause: 0 }
    const id = setInterval(() => {
      s = envelopeStep(s, Math.random())
      el.style.setProperty('--lvl', String(s.lvl))
    }, 70)
    return () => {
      clearInterval(id)
      el.style.removeProperty('--lvl')
    }
  })

  /** A style chip's tip only when its label is cut off (ellipsised); rechecked whenever its grid changes size. */
  function tipWhenCut(grid: HTMLElement) {
    const check = () => {
      for (const b of grid.querySelectorAll<HTMLElement>('.ring-chip')) {
        if (b.scrollWidth > b.clientWidth) b.dataset.tip = b.textContent!.trim()
        else delete b.dataset.tip
      }
    }
    check()
    if (typeof ResizeObserver === 'undefined') return
    const ro = new ResizeObserver(check)
    ro.observe(grid)
    return () => ro.disconnect()
  }

  function load(id: string) {
    loadError = null
    loadProfile(id).then(
      (p) => !gone && !st && p.user.id === app.state.me?.id && (st = new ProfileEditorState(p)),
      (e) => !gone && !st && (loadError = errorText(e)),
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
    st?.dispose()
    clearTimeout(savedTimer)
    for (const p of allPickers) p.cancel()
  })

  function pickFile(kind: Kind, e: Event & { currentTarget: HTMLInputElement }) {
    const file = e.currentTarget.files?.[0]
    e.currentTarget.value = '' // the same file again is a new pick
    if (file) cropping = { file, kind }
  }

  /** Upload the cropped picture and stage it. A refusal leaves the draft as it was. */
  function upload(crop: CropRect) {
    const pick = cropping
    cropping = null
    if (pick && st) void st.upload(pick.kind, pick.file, crop)
  }

  /** Clears the picture, and drops an upload of it still in flight. */
  function remove(kind: Kind) {
    st?.remove(kind)
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

  // The custom card theme's two pickers, in a popover by the + tile (#467)
  let themePop = $state(false)
  function openCustomTheme() {
    customTheme = true
    themePop = true
  }

  // The preview's avatar and banner (#467): a click on either opens that picture's file picker
  const fileInputs: Record<Kind, HTMLInputElement | undefined> = $state({ avatar: undefined, banner: undefined })
  const changePicture = (kind: Kind) => fileInputs[kind]?.click()

  // --- the badges on your card (#467): drag to reorder; add, remove, or go back to the rarest five ---
  const badgeName = (key: string) => {
    const b = held.get(key)
    const d = badgeStore.def(key)
    return d?.name ?? (b?.hidden ? '???' : fallbackName(key))
  }
  function ownShowcase(): string[] {
    // the first edit to the rarest five starts from them
    if (st!.showcase === 'auto') st!.showcase = st!.shown
    return st!.showcase as string[]
  }
  function moveBadge(key: string, to: number) {
    const list = ownShowcase().filter((k) => k !== key)
    list.splice(Math.max(0, Math.min(to, list.length)), 0, key)
    st!.showcase = list
  }
  function toggleBadge(key: string) {
    const list = ownShowcase()
    if (list.includes(key)) st!.showcase = list.filter((k) => k !== key)
    else if (list.length < SHOWCASE_MAX) st!.showcase = [...list, key]
  }
  function badgeKeys(e: KeyboardEvent, key: string) {
    // Alt+← / Alt+→ move the focused badge (the keyboard's drag)
    if (!e.altKey || (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight')) return
    e.preventDefault()
    const i = shownBadges.indexOf(key)
    moveBadge(key, i + (e.key === 'ArrowLeft' ? -1 : 1))
    flushSync()
    document.querySelector<HTMLElement>(`[data-badge="${CSS.escape(key)}"]`)?.focus()
  }

  function showBio(text: string) {
    bioBox?.setText({ text, caret: text.length }, false)
  }

  function reset() {
    if (!st) return
    for (const p of allPickers) p.cancel()
    customTheme = false
    customColour = false
    st.reset()
    showBio(st.saved.bio)
  }

  function save() {
    for (const p of allPickers) p.flush()
    if (!st || st.saving || !changed || invalid || busy) return
    const was = st
    const p = st.save().then(() => {
      if (gone || !was.lastSave || was.saving || was.error) return
      // the preview's custom pickers stay open while their colours were edited meanwhile
      const kept = was.lastSave.kept
      customTheme = customTheme && ('themeTop' in kept || 'themeBottom' in kept)
      customColour = customColour && 'colors' in kept
      if (!('bio' in kept)) showBio(was.profile.card.bio ?? '')
      justSaved = true
      clearTimeout(savedTimer)
      savedTimer = setTimeout(() => (justSaved = false), SAVED_MS)
    })
    settling = p
    void p.finally(() => {
      if (!gone && settling === p) settling = null
    })
  }
</script>

{#snippet picture(kind: Kind, title: string, hash: string | null)}
  <div class="settings-field {kind}-field">
    <span class="settings-label">{title}</span>
    <div class="row">
      <label class="btn btn-secondary upload" class:disabled={uploading[kind]}>Upload
        <input bind:this={fileInputs[kind]} class="file" type="file" accept={ACCEPT} aria-label="Upload {kind}" disabled={uploading[kind]} onchange={(e) => pickFile(kind, e)} />
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
        <!-- the app's segmented row as a tablist (#716): one line at any width; too narrow, it scrolls with a fade -->
        <Segmented label="Profile" tabs={uid} options={TABS.map((t) => ({ value: t.key, label: t.label }))} value={tab} onchange={(v) => (tab = v)} />

        <!-- every tab stays on the page, only the picked one shown: a half-typed bio or an upload carries on across tabs -->
        <div class="panel" role="tabpanel" id="{uid}-panel-about" aria-labelledby="{uid}-tab-about" hidden={tab !== 'about'}>
          {#if nudge}
            <div class="nudge" role="note">
              <span><strong>Pick how your name looks.</strong> Capitals, spaces and emoji are all fine; your username stays for signing in.</span>
              <button type="button" class="btn btn-secondary" onclick={dismissNudge}>Dismiss</button>
            </div>
          {/if}
          <div class="pictures">
            {@render picture('avatar', 'Avatar', draft.avatarHash)}
            {@render picture('banner', 'Banner', draft.bannerHash)}
          </div>
          <div class="two">
            <div class="settings-field name-field">
              <span class="settings-label">Display name</span>
              <!-- counted in characters, as the server counts -->
              <input class="input text" type="text" aria-label="Display name" aria-invalid={!!nameError} placeholder={me.username} bind:value={draft.displayName}
                data-tip="How your name shows everywhere. Not unique: your @{me.username} tells you apart." />
              <!-- the Zalgo easter egg (#209): a soft nudge, not an error; it still saves -->
              {#if nameError}<p class="error" role="alert">{nameError}</p>{:else if isZalgo(draft.displayName)}<small class="zalgo" role="status">Zalgo? Really?</small>{/if}
            </div>
            <div class="settings-field">
              <span class="settings-label">Pronouns</span>
              <!-- counted in characters, as the server counts (maxlength counts UTF-16 units) -->
              <input class="input text" type="text" aria-label="Pronouns" aria-invalid={pronounsOver} placeholder="Add pronouns" bind:value={draft.pronouns} />
              {#if pronounsOver}<p class="error" role="alert">Pronouns can be up to 40 characters.</p>{/if}
            </div>
          </div>
          <div class="settings-field typing-field">
            <span class="settings-label">Typing text</span>
            <!-- "<name> is [ … ]": only what follows "is" is yours to write (#652); counted as the server counts -->
            <div class="typing-row">
              <span class="typing-who name-clip">{typingName}</span><span class="typing-is">is</span>
              <input class="input text" type="text" aria-label="Typing text" aria-invalid={!!typingError} placeholder="typing…" bind:value={draft.typingText}
                data-tip="What shows while you type, in every server and DM. Blank is the plain “is typing…”." />
            </div>
            <div class="chips typing-suggestions" role="group" aria-label="Suggestions">
              {#each TYPING_SUGGESTIONS as t (t)}
                <button type="button" class="toggle-chip" aria-pressed={draft.typingText.trim() === t} onclick={() => (draft!.typingText = t)}>{t}</button>
              {/each}
            </div>
            <p class="typing-preview"><strong class="name-clip">{typingName}</strong> is {draft.typingText.trim() || 'typing…'}</p>
            {#if typingError}<p class="error" role="alert">{typingError}</p>{/if}
          </div>
          <div class="settings-field">
            <span class="settings-label">Bio</span>
            <EmojiBox bind:this={bioBox} bind:text={draft.bio} label="Bio" class="bio" />
            {#if bioLength >= BIO_NEAR}<small class="counter" class:over={bioOver}>{bioLength}/{BIO_MAX}</small>{/if}
            {#if bioBreaksOver}<p class="error" role="alert">Your bio can have at most 4 line breaks.</p>{/if}
          </div>
        </div>

        <div class="panel" role="tabpanel" id="{uid}-panel-card" aria-labelledby="{uid}-tab-card" hidden={tab !== 'card'}>
          <div class="settings-field theme">
            <span class="settings-label">Theme</span>
            <!-- each theme a small card in its own colours; + holds a custom pair, its pickers in a popover -->
            <div class="themes" role="radiogroup" aria-label="Card theme" tabindex="-1" onkeydown={radioKeys}>
              {#each THEME_PRESETS as p (p.key)}
                <button type="button" role="radio" class="mini-card" aria-label={p.label} data-tip={p.label} aria-checked={themeKey === p.key} tabindex={themeKey === p.key ? 0 : -1}
                  style={Object.entries(cardVars(p.top, p.bottom)).map(([k, v]) => `${k}: ${v}`).join('; ')} onclick={() => setTheme(p.top, p.bottom)}>{@render miniCard()}</button>
              {/each}
              <button type="button" role="radio" class="mini-card custom-tile" class:custom={themeKey === null} aria-label="Custom" data-tip="Custom colours" data-menu-toggle
                aria-checked={themeKey === null} tabindex={themeKey === null ? 0 : -1}
                style={themeKey === null ? Object.entries(cardVars(draft.themeTop, draft.themeBottom)).map(([k, v]) => `${k}: ${v}`).join('; ') : undefined}
                onclick={() => (themePop && themeKey === null ? (themePop = false) : openCustomTheme())}>
                {#if themeKey === null}{@render miniCard()}{:else}<Icon name="plus" size={18} />{/if}</button>
            </div>
            {#if themePop && themeKey === null}
              <div class="theme-pop menu-surface pop-in" role="dialog" aria-label="Custom card colours" use:clickOutside={() => (themePop = false)}>
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
            <label class="toggle-row" data-tip="A soft glow in your card's colours around it. Everyone sees your card as you set it.">
              <input type="checkbox" role="switch" class="switch" bind:checked={draft.glow} />
              <span>Glow</span>
            </label>
          </div>
        </div>

        <div class="panel" role="tabpanel" id="{uid}-panel-name" aria-labelledby="{uid}-tab-name" hidden={tab !== 'name'}>
          <div class="settings-field">
            <span class="settings-label">Font</span>
            <!-- your own name, in each font -->
            <div class="fonts" role="radiogroup" aria-label="Name font" tabindex="-1" onkeydown={radioKeys}>
              <button type="button" role="radio" class="font-tile" aria-label="Default" data-tip="Default" aria-checked={draft.font === null} tabindex={fontKnown ? (draft.font === null ? 0 : -1) : 0} onclick={() => (draft!.font = null)}>
                <Name userId={me.id} serverId={null} user={{ ...pu, style: { ...pu.style, font: null, effect: null } }} text={pu.display_name} interactive={false} /></button>
              {#each NAME_FONTS as f (f.key)}
                <button type="button" role="radio" class="font-tile" aria-label={f.label} data-tip={f.label} aria-checked={draft.font === f.key} tabindex={draft.font === f.key ? 0 : -1}
                  data-font={f.family} onclick={() => (draft!.font = f.key)}>
                  <Name userId={me.id} serverId={null} user={{ ...pu, style: { ...pu.style, font: f.key, effect: null } }} text={pu.display_name} interactive={false} /></button>
              {/each}
            </div>
          </div>

          <div class="settings-field colour" bind:this={colourField}>
            <span class="settings-label">Colour</span>
            <div class="colours" role="radiogroup" aria-label="Name colour" tabindex="-1" onkeydown={colourKeys}>
              <div class="colour-group">
                <span class="group-label">Solid</span>
                <div class="swatches">
                  <!-- no colour: a struck-through swatch, sized like the presets beside it -->
                  <button type="button" role="radio" class="swatch round none" aria-label="Default" data-tip="Default colour" aria-checked={colourKey === 'default'} tabindex={colourKey === 'default' ? 0 : -1} onclick={() => setColours([])}></button>
                  {#each NAME_SOLIDS as p (p.key)}
                    <button type="button" role="radio" class="swatch round" aria-label={p.label} data-tip={p.label} aria-checked={colourKey === `solid:${p.key}`} tabindex={colourKey === `solid:${p.key}` ? 0 : -1}
                      style:background={p.colors[0]} onclick={() => setColours(p.colors)}></button>
                  {/each}
                </div>
              </div>
              <div class="colour-group">
                <span class="group-label">Gradient</span>
                <div class="swatches">
                  {#each NAME_GRADIENTS as p (p.key)}
                    <button type="button" role="radio" class="swatch round" aria-label="{p.label} gradient" data-tip={p.label} aria-checked={colourKey === `gradient:${p.key}`} tabindex={colourKey === `gradient:${p.key}` ? 0 : -1}
                      style:background="linear-gradient(135deg, {p.colors[0]}, {p.colors[1]})" onclick={() => setColours(p.colors)}></button>
                  {/each}
                  <button type="button" role="radio" class="swatch round pick" aria-label="Custom" data-tip="Custom colour" aria-checked={colourKey === 'custom'} tabindex={colourKey === 'custom' ? 0 : -1} onclick={pickCustomColour}><Icon name="plus" size={14} /></button>
                </div>
              </div>
            </div>
            {#if colourKey === 'custom'}
              <Segmented label="Custom colour" value={draft.colors.length >= 2 ? 'gradient' : 'solid'} onchange={setCustomCount}
                options={[{ value: 'solid', label: 'Solid' }, { value: 'gradient', label: 'Gradient' }]} />
              <div class="row">
                {#each draft.colors as c, i}
                  <input type="color" aria-label={draft.colors.length >= 2 ? (i === 0 ? 'First colour' : 'Second colour') : 'Colour'} value={c}
                    data-tip={lightened ? 'Lightened so it reads on dark backgrounds' : undefined}
                    oninput={(e) => pickers.colours[i].push(e.currentTarget.value)} onchange={() => pickers.colours[i].flush()} />
                {/each}
              </div>
            {/if}
          </div>

          <div class="settings-field">
            <span class="settings-label">Effect</span>
            <!-- each effect drawn on your own name style; hover or focus one to see it move (#468) -->
            <div class="chips" role="radiogroup" aria-label="Name effect" tabindex="-1" onkeydown={radioKeys}>
              <button type="button" role="radio" class="chip toggle-chip" aria-checked={draft.effect === null} tabindex={effectKnown ? (draft.effect === null ? 0 : -1) : 0} onclick={() => (draft!.effect = null)}>None</button>
              {#each NAME_EFFECTS as e (e.key)}
                <button type="button" role="radio" class="chip toggle-chip fx-chip" aria-checked={draft.effect === e.key} tabindex={draft.effect === e.key ? 0 : -1} aria-label={e.label} data-tip={e.moves}
                  data-effect={e.key} onclick={() => (draft!.effect = e.key)} onpointerenter={() => (effectHot = e.key)} onpointerleave={() => (effectHot = null)}
                  onfocus={() => (effectHot = e.key)} onblur={() => (effectHot = null)}>
                  <Name userId={me.id} serverId={null} user={{ ...pu, style: { ...pu.style, effect: e.key } }} text={e.label} interactive={false} play={effectHot === e.key} />
                </button>
              {/each}
            </div>
          </div>
        </div>

        <div class="panel" role="tabpanel" id="{uid}-panel-ring" aria-labelledby="{uid}-tab-ring" hidden={tab !== 'ring'}>
          <div class="ring-head">
            <h3 class="settings-section-title">Speaking ring</h3>
            <p class="muted">How your avatar lights up when you talk. Everyone in the call sees it.</p>
          </div>
          <!-- the preview on the left, the controls on the right; stacked when there's no room beside it -->
          <div class="ring-row">
            <div class="ring-preview">
              <div class="ring-stage" bind:this={ringStage}>
                <Avatar id={me.id} name={pu.display_name} user={pu} size={PREVIEW_D} />
                <SpeakingRing userId={me.id} d={PREVIEW_D} speaking place="preview" style={draft.ringStyle} color={draft.ringColor} user={pu} />
              </div>
              <p class="ring-caption">{previewCaption}</p>
            </div>
            <div class="ring-controls">
              <div class="settings-field">
                <span class="settings-label">Style</span>
                <!-- one radiogroup across both groups: the arrows run from Viewfinder on to Liquid -->
                <div class="ring-styles" role="radiogroup" aria-label="Ring style" tabindex="-1" onkeydown={radioKeys}>
                  {#each RING_GROUPS as g (g.key)}
                    <div class="ring-group" role="group" aria-labelledby="{uid}-ring-{g.key}">
                      <span class="ring-group-label" id="{uid}-ring-{g.key}">{g.label}</span>
                      <div class="ring-grid" {@attach tipWhenCut}>
                        {#each g.styles as r (r.key)}
                          <button type="button" role="radio" class="toggle-chip ring-chip" aria-checked={draft.ringStyle === r.key} tabindex={draft.ringStyle === r.key ? 0 : -1}
                            onclick={() => (draft!.ringStyle = r.key)}>{r.label}</button>
                        {/each}
                      </div>
                    </div>
                  {/each}
                </div>
              </div>
              <div class="settings-field">
                <span class="settings-label">Colour</span>
                <div class="ring-swatches" role="radiogroup" aria-label="Ring colour" tabindex="-1" onkeydown={radioKeys}>
                  {#each RING_COLORS as c (c.key)}
                    <button type="button" role="radio" class="swatch round ring-swatch" aria-label={c.label} data-tip={c.key === 'profile' ? profileTip : c.label}
                      aria-checked={draft.ringColor === c.key} tabindex={draft.ringColor === c.key ? 0 : -1} style:--swatch={swatchCss(c.key)} onclick={() => (draft!.ringColor = c.key)}></button>
                  {/each}
                </div>
              </div>
              <p class="muted ring-note">With reduced motion on, everyone sees the still version of each ring. In big calls some rings show still to keep things smooth.</p>
            </div>
          </div>
        </div>

        <div class="panel" role="tabpanel" id="{uid}-panel-badges" aria-labelledby="{uid}-tab-badges" hidden={tab !== 'badges'}>
          {#if held.size === 0}
            <p class="muted">No badges yet. The ones you earn can go on your card.</p>
          {:else}
            <div class="settings-field">
              <span class="settings-label">On your card</span>
              <!-- drag to reorder (Alt+arrows from the keyboard); click one to take it off -->
              <ol class="shown-badges" aria-label="Badges on your card" use:railDrag={{ item: 'badge', onDrop: moveBadge, disabled: shownBadges.length < 2 }}>
                {#each shownBadges as key (key)}
                  {@const b = held.get(key)}
                  {#if b}
                    {@const d = badgeStore.def(key)}
                    <li>
                      <button type="button" class="badge-slot" data-badge={key} aria-label="{badgeName(key)}: take off your card" data-tip="{badgeName(key)} · drag to move, click to take off"
                        onkeydown={(e) => badgeKeys(e, key)} onclick={() => toggleBadge(key)}>
                        <BadgeIcon icon={d?.icon ?? '🏆'} art={d?.art} rarity={b.rarity} tier={b.tier} tiers={d?.tiers.length ?? 1} first={b.first_earner} hidden={b.hidden} size="show" label={badgeName(key)} />
                      </button>
                    </li>
                  {/if}
                {:else}
                  <li class="muted none">None on your card</li>
                {/each}
              </ol>
              <div class="row">
                <button type="button" class="btn btn-secondary" disabled={showcaseAuto} data-tip="Your five rarest, following new ones as you earn them" onclick={() => st && (st.showcase = 'auto')}>Rarest five</button>
                {#if showcaseAuto}<span class="muted">Showing your rarest five</span>{/if}
              </div>
            </div>
            <div class="settings-field">
              <span class="settings-label">Your badges</span>
              <div class="all-badges">
                {#each profile.badges as b (b.key)}
                  {@const d = badgeStore.def(b.key)}
                  {@const on = shownBadges.includes(b.key)}
                  <button type="button" class="badge-pick" aria-pressed={on} disabled={!on && shownBadges.length >= SHOWCASE_MAX}
                    aria-label={on ? `${badgeName(b.key)}: take off your card` : `${badgeName(b.key)}: add to your card`} data-tip={badgeName(b.key)} onclick={() => toggleBadge(b.key)}>
                    <BadgeIcon icon={d?.icon ?? '🏆'} art={d?.art} rarity={b.rarity} tier={b.tier} tiers={d?.tiers.length ?? 1} first={b.first_earner} hidden={b.hidden} size="show" label={badgeName(b.key)} />
                  </button>
                {/each}
              </div>
            </div>
          {/if}
        </div>
      </div>

      <div class="preview" aria-label="Preview">
        <ProfileCard user={pu} card={pc} badges={profile.badges} showcase={shownBadges} stats={profile.stats} serverId={null} preview
          onEditAvatar={() => changePicture('avatar')} onEditBanner={() => changePicture('banner')} />
        <div class="line">
          <Avatar id={me.id} name={pu.display_name} user={pu} size={36} />
          <div class="said">
            <Name userId={me.id} serverId={null} user={pu} text={pu.display_name} interactive={false} play />
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

{#snippet miniCard()}
  <!-- a card in miniature: banner, avatar and two lines of text, in the theme's own colours -->
  <span class="mc-banner"></span><span class="mc-avatar"></span><span class="mc-line"></span><span class="mc-line short"></span>
{/snippet}

{#if cropping}
  {#key cropping}
    <CropDialog file={cropping.file} kind={cropping.kind} onApply={upload} onCancel={() => (cropping = null)} />
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
  /* an effect's chip shows it on your name style, a little larger so the effect reads */
  .fx-chip { font-size: 15px; font-weight: 700; }
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
  /* typing text (#652): "<name> is [ … ]" on one row that shrinks to any width, the box first, then a long name */
  .typing-row { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .typing-who { flex: 0 1 auto; max-width: 45%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 600; }
  .typing-is { flex: none; color: var(--text-2); }
  .typing-row .input { flex: 1 1 0; min-width: 0; }
  .typing-suggestions { flex-wrap: wrap; }
  .typing-preview { margin: 0; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 12px; color: var(--text-3); }
  .typing-preview strong { color: var(--text); }
  /* the speaking ring (#651 §7.1): nothing spills at any width. The preview panel keeps its 200px; with no room beside
     it, the row wraps and the controls go under it. The chips ellipsise in their cells, the swatches wrap. */
  .ring-head { display: flex; flex-direction: column; gap: 2px; margin-bottom: 16px; }
  .ring-head p { margin: 0; }
  .ring-row { display: flex; flex-wrap: wrap; align-items: flex-start; gap: 20px; }
  .ring-preview { flex: 0 0 200px; width: 200px; height: 200px; box-sizing: border-box; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 8px; padding: 8px; background: var(--bg-1); border: 1px solid var(--bg-3); border-radius: var(--radius-xl); overflow: hidden; }
  /* the avatar's own box; its margin reserves the ring box's 0.16 D outset, and it's the ring's stacking context */
  .ring-stage { position: relative; flex: none; width: 96px; height: 96px; margin: calc(96px * .16); isolation: isolate; border-radius: 50%; }
  .ring-caption { margin: 0; max-width: 100%; text-align: center; overflow-wrap: anywhere; font-size: 12px; color: var(--text-3); }
  .ring-controls { flex: 1 1 240px; min-width: 0; display: flex; flex-direction: column; gap: 16px; }
  .ring-styles { display: flex; flex-direction: column; gap: 12px; }
  .ring-group { display: flex; flex-direction: column; gap: 6px; min-width: 0; }
  .ring-group-label { font-size: 12px; color: var(--text-3); }
  .ring-grid { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 6px; }
  .ring-chip { display: block; min-width: 0; text-align: center; line-height: 26px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ring-swatches { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
  .ring-swatch { flex: none; background: var(--swatch); }
  .ring-note { margin: 0; }
</style>
