<script lang="ts">
  // The profile editor (#727): a large modal with the menu on the left, the section's centre and right columns, and a
  // Reset / Save footer. Everything profile-related is staged in one ProfileEditorState until Save. App appearance
  // saves instantly as it always did, so it sits outside the profile load gate (it works when your profile can't
  // load) and hides Reset and Save.
  //
  // Closing guard: ×, Escape and a click outside all go through `requestClose`, which asks once when there are
  // unsaved changes. A layer nested inside the editor (a popover) keeps Escape and the outside click for itself:
  // Escape is ignored when the key event is already `defaultPrevented` (the `modal` action does this; a popover
  // handles its Escape and calls preventDefault), and an outside click on the backdrop is ignored while any element
  // marked `data-editor-layer` is on the page (a popover marks its root with it and closes itself on that click).
  import { untrack } from 'svelte'
  import Icon from '../Icon.svelte'
  import Loading from '../Loading.svelte'
  import CardEditor from './CardEditor.svelte'
  import AppearanceSettings from '../AppearanceSettings.svelte'
  import EditorMenu from './EditorMenu.svelte'
  import { modal } from '../../lib/actions'
  import { closeProfileEditor, profileEditor, ProfileEditorState, type EditorSection } from '../../lib/profileEditor.svelte'
  import { previewCard, previewUser } from '../../lib/profileDraft'
  import { app, loadProfile } from '../../lib/store.svelte'
  import { errorText } from '../../lib/tauri'

  /** `state`: a working copy already in hand (skips the profile load); the editor loads your profile itself without one. */
  let { state: given = null }: { state?: ProfileEditorState | null } = $props()

  const SAVED_MS = 2000
  let st = $state.raw<ProfileEditorState | null>(untrack(() => given))
  let loadError = $state<string | null>(null)
  let asking = $state(false)
  let justSaved = $state(false)
  let savedTimer: ReturnType<typeof setTimeout> | undefined
  let gone = false

  const me = $derived(app.state.me)
  const section = $derived(profileEditor.section)
  const unsaved = $derived(!!st && (st.dirty || st.busy))
  const pu = $derived(me && st ? previewUser(me, st.draft) : null)
  const pc = $derived(st ? previewCard(st.draft) : null)

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
    if (!id || id === loadedFor || st) return
    loadedFor = id
    untrack(() => load(id))
  })
  $effect(() => () => {
    gone = true
    st?.dispose()
    clearTimeout(savedTimer)
  })

  function requestClose() {
    if (st?.saving) return // a Save is on its way: it may yet fail and keep the draft
    if (unsaved) asking = true
    else closeProfileEditor()
  }
  /** Escape while asking means "Keep editing". */
  const escape = () => (asking ? (asking = false) : requestClose())
  function backdrop() {
    if (document.querySelector('[data-editor-layer]')) return
    requestClose()
  }
  function discard() {
    asking = false
    st?.reset()
    closeProfileEditor()
  }
  const pick = (s: EditorSection) => (profileEditor.section = s)
  const focusOnShow = (el: HTMLElement) => el.focus()

  async function save() {
    if (!st) return
    const was = st
    await was.save()
    if (gone || was.error || !was.lastSave) return
    justSaved = true
    clearTimeout(savedTimer)
    savedTimer = setTimeout(() => (justSaved = false), SAVED_MS)
  }
</script>

<div class="overlay modal-overlay fade-in">
  <div class="backdrop modal-backdrop" role="presentation" data-backdrop onclick={backdrop}></div>
  <div class="modal pop-in" role="dialog" aria-modal="true" aria-label="Edit profile" tabindex="-1" use:modal={escape}>
    <header>
      <h1>Edit profile</h1>
      <button type="button" class="icon-btn round modal-close" aria-label="Close editor" onclick={requestClose}><Icon name="x" size={16} /></button>
    </header>
    {#if asking}
      <div class="confirm" role="alert">
        <span>Discard your profile changes?</span>
        <button type="button" class="btn btn-secondary" {@attach focusOnShow} onclick={() => (asking = false)}>Keep editing</button>
        <button type="button" class="btn btn-danger" onclick={discard}>Discard</button>
      </div>
    {/if}
    <div class="body">
      <EditorMenu {section} onPick={pick} />
      <div class="stage" data-section={section}>
        {#if section === 'appearance'}
          <div class="appearance"><AppearanceSettings /></div>
        {:else if loadError}
          <p class="error" role="alert">{loadError} <button type="button" class="btn btn-secondary retry" onclick={() => me && load(me.id)}>Retry</button></p>
        {:else if !st || !me || !pu || !pc}
          <Loading />
        {:else}
          <!-- The inline-editing card; Task 4 adds "Where you show up" and the tile column.
               Card stats shows only the card until Task 12 adds its right column. -->
          <div class="centre">
            <CardEditor state={st} />
          </div>
        {/if}
      </div>
    </div>
    {#if section !== 'appearance'}
      <footer>
        {#if st?.error}<p class="error" role="alert">{st.error}</p>{/if}
        <span class="state">
          {#if st?.dirty}Unsaved changes{:else if justSaved}<span class="ok" role="status">Saved</span>{/if}
        </span>
        <button type="button" class="btn btn-lg btn-secondary" disabled={!st || !unsaved || st.saving} onclick={() => st?.reset()}>Reset</button>
        <button type="button" class="btn btn-lg btn-primary" disabled={!st || !st.dirty || st.saving || st.invalid || st.busy} onclick={save}>Save</button>
      </footer>
    {/if}
  </div>
</div>

<style>
  .overlay { --editor-margin: 24px; position: fixed; inset: 0; z-index: 20; padding: var(--editor-margin); }
  /* as large as the window allows, an even margin all round, shrinking with the window */
  .modal { width: min(1280px, calc(100vw - 2 * var(--editor-margin))); max-width: calc(100vw - 2 * var(--editor-margin)); height: min(880px, calc(100vh - 2 * var(--editor-margin))); max-height: calc(100vh - 2 * var(--editor-margin)); gap: 0; padding: 0; background: var(--bg-1); overflow: hidden; }
  header { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 14px 16px 14px 20px; border-bottom: 1px solid var(--line-soft); flex-shrink: 0; }
  h1 { margin: 0; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 16px; font-weight: 600; }
  .confirm { display: flex; align-items: center; flex-wrap: wrap; gap: 8px; margin: 12px 16px 0; padding: 10px 14px; border-radius: var(--radius-lg); background: var(--bg-3); border: 1px solid var(--warn); font-weight: 600; flex-shrink: 0; }
  .confirm span { margin-right: auto; }
  .body { flex: 1; min-height: 0; display: flex; }
  .stage { flex: 1; min-width: 0; min-height: 0; overflow-y: auto; overflow-x: hidden; padding: 24px; }
  .centre { display: flex; justify-content: center; min-width: 0; }
  .centre > :global(*) { max-width: 100%; }
  .appearance { min-width: 0; max-width: 720px; margin: 0 auto; }
  footer { display: flex; align-items: center; justify-content: flex-end; flex-wrap: wrap; gap: 8px; padding: 12px 20px; border-top: 1px solid var(--line-soft); flex-shrink: 0; }
  footer .error { flex-basis: 100%; }
  .state { margin-right: auto; color: var(--warn-text); font-weight: 600; font-size: 13px; }
  .state .ok { color: var(--ok-text); }
  .error { margin: 0; min-width: 0; overflow-wrap: anywhere; color: var(--danger-text); font-size: 13px; }
  .retry { height: 28px; margin-left: 8px; }
</style>
