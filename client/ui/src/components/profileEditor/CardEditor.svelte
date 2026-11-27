<script lang="ts">
  // The card, edited in place (#727): the display name, pronouns and bio are fields on the card itself, the avatar and
  // banner open the Change / Remove menu (ImageMenu). Everything goes into the editor's draft; nothing saves until Save.
  // A problem shows as a short line under its field; an upload that fails, under the card.
  //
  // Bio: raw text while it has focus (with an emoji button at its corner), formatted when it doesn't. Both states are
  // the same box, the same fixed minimum and maximum height, so the card doesn't jump between them.
  import { tick } from 'svelte'
  import EmojiBox from '../EmojiBox.svelte'
  import EmojiPopover from '../EmojiPopover.svelte'
  import Icon from '../Icon.svelte'
  import ProfileCard from '../ProfileCard.svelte'
  import ImageMenu from './ImageMenu.svelte'
  import Name from '../Name.svelte'
  import { cardTheme, nameStop } from '../../lib/cardTheme'
  import { cardLook } from '../../lib/theme.svelte'
  import { baseName } from '../../lib/selectors'
  import { customToken, savedTone, withSkin, type Std } from '../../lib/emoji'
  import { insertAt } from '../../lib/emojiComplete'
  import { renderMarkdown } from '../../lib/markdown'
  import { BIO_LINE_BREAKS_MAX, BIO_MAX, charCount, isZalgo, nameProblem, normalBio, previewCard, previewStats, previewUser, PRONOUNS_MAX } from '../../lib/profileDraft'
  import type { ProfileEditorState } from '../../lib/profileEditor.svelte'
  import type { CustomEmoji } from '../../lib/protocol/CustomEmoji'
  import { app } from '../../lib/store.svelte'

  let { state: st }: { state: ProfileEditorState } = $props()

  const BIO_NEAR = 150
  const me = $derived(app.state.me)
  const pu = $derived(me ? previewUser(me, st.draft) : null)
  const pc = $derived(previewCard(st.draft))

  const nameError = $derived(nameProblem(st.draft.displayName))
  const nameTip = $derived(`How your name shows everywhere. Not unique: your @${me?.username ?? ''} tells you apart.`)
  const pronounsOver = $derived(charCount(st.draft.pronouns.trim()) > PRONOUNS_MAX)
  const bio = $derived(normalBio(st.draft.bio))
  const bioLength = $derived(charCount(bio))
  const bioOver = $derived(bioLength > BIO_MAX)
  const bioBreaksOver = $derived((bio.match(/\n/g)?.length ?? 0) > BIO_LINE_BREAKS_MAX)
  const bioHtml = $derived(bio ? renderMarkdown(bio, [], app.state.customEmoji) : '')

  // the name as it will look (font, colours, effect), drawn as the card draws it
  const theme = $derived(cardTheme(pc.theme_top, pc.theme_bottom, cardLook()))
  const nameBg = $derived(nameStop(theme.vars['--card-top'], theme.vars['--card-bottom'], theme.light))
  let nameEditing = $state(false)
  let nameInput = $state<HTMLInputElement>()
  async function editName() {
    nameEditing = true
    await tick()
    nameInput?.focus()
  }

  let editing = $state(false)
  let bioBox = $state<EmojiBox>()
  let emojiBtn = $state<HTMLButtonElement>()
  let picking = $state(false)

  async function startEditing() {
    editing = true
    await tick()
    bioBox?.focus(st.draft.bio.length)
  }
  // Leaving the field formats it, unless focus is only going to the emoji button or the picker it opened.
  function leave(e: FocusEvent) {
    if ((e.target as HTMLElement).classList.contains('bio-view')) return // the view is being swapped for the box
    if (picking || (e.currentTarget as HTMLElement).contains(e.relatedTarget as Node | null)) return
    editing = false
  }
  function pickEmoji(e: Std | CustomEmoji, { keep }: { keep: boolean }) {
    if (!keep) picking = false
    const sel = bioBox?.selection() ?? { start: st.draft.bio.length, end: st.draft.bio.length }
    const r = insertAt(st.draft.bio, sel.start, sel.end, 'u' in e ? withSkin(e, savedTone()) : customToken(e))
    void bioBox?.setText(r, !keep)
  }
  // Escape while the picker is open closes it, ahead of the editor's own Escape (which then sees it handled)
  $effect(() => {
    if (!picking) return
    const key = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      e.preventDefault()
      closePicker()
    }
    window.addEventListener('keydown', key, true)
    return () => window.removeEventListener('keydown', key, true)
  })
  function closePicker() {
    picking = false
    bioBox?.focus()
  }
</script>

{#snippet nameField()}
  <div class="name-box">
    {#if nameEditing}
      <input bind:this={nameInput} class="name-in field" type="text" aria-label="Display name" data-tip={nameTip} aria-invalid={!!nameError} placeholder={me?.username} bind:value={st.draft.displayName}
        onblur={() => (nameEditing = false)} />
    {:else if pu}
      <div class="name-view field" role="button" tabindex="0" aria-label="Edit display name" data-tip={nameTip} onclick={editName} onfocus={editName}
        onkeydown={(e) => { if (e.key === 'Enter') { e.preventDefault(); void editName() } }}>
        <Name userId={pu.id} serverId={null} text={baseName(pu)} interactive={false} size="card" bg={nameBg} user={pu} play />
      </div>
    {/if}
  </div>
  {#if nameError}<p class="err" role="alert">{nameError}</p>{:else if isZalgo(st.draft.displayName)}<small class="zalgo" role="status">Zalgo? Really?</small>{/if}
{/snippet}

{#snippet pronounsField()}
  <input class="pro-in field" type="text" aria-label="Pronouns" aria-invalid={pronounsOver} placeholder="pronouns" bind:value={st.draft.pronouns} />
  {#if pronounsOver}<p class="err wide" role="alert">Pronouns can be up to {PRONOUNS_MAX} characters.</p>{/if}
{/snippet}

{#snippet bioField()}
  <div class="bio-field" role="presentation" onfocusout={leave}>
    {#if editing}
      <EmojiBox bind:this={bioBox} bind:text={st.draft.bio} label="Bio" placeholder="Add a bio" class="bio-box bio-edit" />
      <button bind:this={emojiBtn} type="button" class="emoji-btn" aria-label="Emoji" aria-haspopup="dialog" aria-expanded={picking} data-menu-toggle
        onmousedown={(e) => e.preventDefault()} onclick={() => (picking = !picking)}><Icon name="smile" size={18} /></button>
    {:else}
      <!-- renderMarkdown output is DOMPurify-sanitised (the message renderer's allow-list) -->
      <div class="bio-view bio-box" class:empty={!bioHtml} role="button" tabindex="0" aria-label="Edit bio" data-placeholder="Add a bio"
        onclick={startEditing} onfocus={startEditing} onkeydown={(e) => { if (e.key === 'Enter') { e.preventDefault(); void startEditing() } }}>{@html bioHtml}</div>
    {/if}
    {#if bioLength >= BIO_NEAR || bioOver}<small class="counter" class:over={bioOver}>{bioLength}/{BIO_MAX}</small>{/if}
    {#if bioBreaksOver}<p class="err" role="alert">Your bio can have at most {BIO_LINE_BREAKS_MAX} line breaks.</p>{/if}
  </div>
  {#if picking && emojiBtn}
    <div class="layer" data-editor-layer><EmojiPopover anchor={emojiBtn} onPick={pickEmoji} onClose={closePicker} /></div>
  {/if}
{/snippet}

{#snippet bannerPicture()}<ImageMenu kind="banner" state={st} />{/snippet}
{#snippet avatarPicture()}<ImageMenu kind="avatar" state={st} />{/snippet}

{#if me && pu}
  <div class="card-editor">
    <ProfileCard user={pu} card={pc} badges={st.profile.badges} showcase={st.shown} stats={previewStats(st.profile.stats, st.draft)} serverId={null} preview
      editable={{ name: nameField, pronouns: pronounsField, bio: bioField, avatar: avatarPicture, banner: bannerPicture }} />
    {#if st.uploadError}<p class="up-err" role="alert">{st.uploadError}</p>{/if}
  </div>
{/if}

<style>
  .card-editor { display: flex; flex-direction: column; align-items: center; gap: 8px; min-width: 0; max-width: 100%; }
  .up-err { margin: 0; max-width: calc(300px * 1.12); text-align: center; font-size: 13px; color: var(--danger-text); overflow-wrap: anywhere; }
  .layer { display: contents; }

  /* a field on the card: plain until hovered or focused; a dashed outline invites, a solid accent one says "typing".
     The outline takes no room, so nothing moves. */
  .field { box-sizing: border-box; min-width: 0; border: 0; border-radius: var(--radius-sm); background: transparent; color: inherit; font: inherit; outline: 1px dashed transparent; outline-offset: 2px; }
  .field::placeholder { color: var(--card-text-2); opacity: 1; }
  .field:hover, .field:focus-visible { outline-color: var(--card-text-2); }
  .field:focus { outline: 2px solid var(--focus-ring); outline-offset: 2px; }
  /* the name as the card sets it (Name's card size: 20px x --k, bold), clipped with an ellipsis when it doesn't fit */
  .name-box { height: calc(20px * var(--k) * 1.3 + 4px); min-width: 0; }
  .name-view { display: flex; align-items: center; height: 100%; cursor: text; }
  .name-view :global(.name) { font-weight: 700; }
  .name-in { height: 100%; width: 100%; padding: 2px 0; font-size: calc(20px * var(--k)); font-weight: 700; line-height: 1.3; text-overflow: ellipsis; overflow: hidden; white-space: nowrap; }
  .pro-in { flex: 1 1 6em; padding: 0; color: var(--card-text-2); text-overflow: ellipsis; overflow: hidden; white-space: nowrap; }
  .err { margin: 2px 0 0; font-size: calc(12px * var(--k)); color: var(--card-danger); overflow-wrap: anywhere; }
  .zalgo { display: block; margin: 2px 0 0; font-size: calc(12px * var(--k)); color: var(--card-text-2); overflow-wrap: anywhere; }
  .err.wide { flex: 1 0 100%; }

  /* the bio box, raw or formatted: the same box with the same fixed minimum and maximum (3 to 5 lines), scrolling past it */
  .bio-field { position: relative; min-width: 0; }
  .bio-field :global(.bio-box) { display: block; box-sizing: border-box; width: 100%; min-height: calc(20px * 3 + 18px); max-height: calc(20px * 5 + 18px); overflow-y: auto; padding: 8px 40px 8px 12px; border: 1px solid var(--card-edge); border-radius: var(--radius-md); background: var(--card-field); color: var(--card-text); font-size: calc(13px * var(--k)); line-height: 20px; overflow-wrap: anywhere; outline: 1px dashed transparent; outline-offset: 2px; }
  .bio-field :global(.bio-box:hover), .bio-field :global(.bio-box:focus-visible) { outline-color: var(--card-text-2); }
  .bio-field :global(.bio-edit) { outline: 2px solid var(--focus-ring); }
  .bio-field :global(.bio-edit.empty::before) { color: var(--card-text-2); }
  .bio-view { cursor: text; }
  .bio-view.empty::before { content: attr(data-placeholder); color: var(--card-text-2); }
  .bio-view :global(p) { margin: 0; }
  .bio-view :global(p + p) { margin-top: 4px; }
  .emoji-btn { position: absolute; right: 6px; bottom: 6px; display: grid; place-items: center; width: 28px; height: 28px; padding: 0; border: 0; border-radius: var(--radius-sm); background: none; color: var(--card-text-2); cursor: pointer; }
  .emoji-btn:hover, .emoji-btn[aria-expanded='true'] { background: var(--card-hover); color: var(--card-text); }
  .counter { display: block; margin-top: 2px; text-align: right; font-size: calc(12px * var(--k)); color: var(--card-text-2); }
  .counter.over { color: var(--card-danger); }
</style>
