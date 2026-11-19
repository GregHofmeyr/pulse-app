<script lang="ts">
  // Settings › About (#506): which Pulse this is and where it's connected, Check for updates (the updater's own
  // answer: up to date, or the restart, as the launch check offers it; elsewhere the package manager's), and this
  // version's patch notes (the title bar's, opened over Settings).
  import { onMount } from 'svelte'
  import Icon from './Icon.svelte'
  import PatchNotes from './PatchNotes.svelte'
  import UpdateSettings from './UpdateSettings.svelte'
  import { loadServerOrigin, serverOrigin } from '../lib/invitePreviews.svelte'
  import { markPatchNotesSeen } from '../lib/patchNotesSeen.svelte'
  import { patchNotes, previousPatchNotes } from '../patchNotes'

  onMount(loadServerOrigin)
  /** The server as people would write it: its host (and port, if any), not the whole URL. */
  const server = $derived.by(() => {
    const o = serverOrigin.origin
    if (!o) return null
    try {
      return new URL(o).host
    } catch {
      return o
    }
  })
  let notesOpen = $state(false)
  function openNotes() {
    notesOpen = true
    markPatchNotesSeen(patchNotes.id)
  }
</script>

<section class="settings-pane">
  <h2>About</h2>
  <!-- the version (the app's own, as the updater reads it) and Check for updates -->
  <UpdateSettings />
  <dl class="facts">
    <div><dt>Connected to</dt><dd data-server>{server ?? '…'}</dd></div>
  </dl>
  <button type="button" class="btn btn-secondary notes" onclick={openNotes}><Icon name="scroll" size={14} /> What's new in {patchNotes.version}</button>
</section>

{#if notesOpen}<PatchNotes notes={patchNotes} previous={previousPatchNotes} onClose={() => (notesOpen = false)} />{/if}

<style>
  .facts { margin: 0; display: grid; gap: 8px; }
  .facts div { display: flex; gap: 12px; font-size: 14px; }
  dt { width: 110px; flex-shrink: 0; color: var(--text-3); }
  dd { margin: 0; min-width: 0; color: var(--text); font-weight: 600; overflow-wrap: anywhere; }
  .notes { align-self: flex-start; }
</style>
