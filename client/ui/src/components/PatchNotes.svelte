<script lang="ts">
  import Icon from './Icon.svelte'
  import { modal } from '../lib/actions'
  import type { PatchNotes } from '../patchNotes'

  // What's changed in this release (#207), from the title bar. Empty sections are left out. Coming soon (#224) is a
  // light roadmap after Quality of life, styled a touch softer, at most five bullets. The release before (#235) sits
  // below as one closed line; opened, it shows its changes the same way (not its stale roadmap or sign-off).
  const SOON_MAX = 5
  let { notes, previous = null, onClose }: { notes: PatchNotes; previous?: PatchNotes | null; onClose: () => void } = $props()

  const changes = (n: PatchNotes): { title: string; items: string[]; soft?: boolean }[] => [
    { title: 'New', items: n.new },
    { title: 'Bug fixes', items: n.fixes },
    { title: 'Quality of life', items: n.qol },
  ]
  const sections = $derived(
    [...changes(notes), { title: 'Coming soon', items: (notes.soon ?? []).slice(0, SOON_MAX), soft: true }].filter((s) => s.items.length > 0),
  )
  const previousSections = $derived(previous ? changes(previous).filter((s) => s.items.length > 0) : [])
</script>

<div class="overlay modal-overlay fade-in">
  <div class="backdrop modal-backdrop" role="presentation" data-backdrop onclick={onClose}></div>
  <div class="modal pop-in" role="dialog" aria-modal="true" aria-label="Patch notes" tabindex="-1" use:modal={onClose}>
    <div class="top">
      <div class="heading">
        <h1 class="modal-title">Patch notes</h1>
        <p class="date">Pulse {notes.version} · {notes.date}</p>
      </div>
      <button type="button" class="icon-btn round modal-close" aria-label="Close" onclick={onClose}><Icon name="x" size={16} /></button>
    </div>
    {#each sections as s (s.title)}
      <section class:soon={s.soft}>
        <h2>{s.title}</h2>
        <ul>
          {#each s.items as item, i (i)}<li>{item}</li>{/each}
        </ul>
      </section>
    {/each}
    {#if notes.message}<p class="message">{notes.message}</p>{/if}
    {#if previous}
      <details class="previous">
        <summary><Icon name="chevronRight" size={14} />Previous update ({previous.version})</summary>
        <div class="previous-body">
          <p class="date">{previous.date}</p>
          {#each previousSections as s (s.title)}
            <section>
              <h2>{s.title}</h2>
              <ul>
                {#each s.items as item, i (i)}<li>{item}</li>{/each}
              </ul>
            </section>
          {/each}
        </div>
      </details>
    {/if}
    <div class="actions">
      <button type="button" class="btn btn-lg btn-primary" onclick={onClose}>Close</button>
    </div>
  </div>
</div>

<style>
  .overlay { position: fixed; inset: 0; z-index: 30; }
  .modal { width: min(480px, 100%); }
  .top { display: flex; align-items: flex-start; justify-content: space-between; gap: 12px; }
  .heading { min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .date { margin: 0; font-size: 12px; color: var(--text-3); }
  section { display: flex; flex-direction: column; gap: 6px; }
  h2 { margin: 0; font-size: 11px; font-weight: 700; letter-spacing: .06em; text-transform: uppercase; color: var(--text-3); }
  ul { margin: 0; padding-left: 18px; display: flex; flex-direction: column; gap: 4px; }
  li { font-size: 13px; line-height: 1.45; color: var(--text-2); overflow-wrap: anywhere; }
  li::marker { color: var(--text-3); }
  /* Coming soon is a roadmap, not a promise: a touch softer than the shipped sections. */
  .soon li { color: var(--text-row); }
  .message { margin: 4px 0 0; padding-top: 12px; border-top: 1px solid var(--line-soft); font-size: 13px; line-height: 1.5; color: var(--text-3); white-space: pre-line; }
  /* The release before (#235): one quiet line under a divider, like the sign-off's, until opened. */
  .previous { padding-top: 12px; border-top: 1px solid var(--line-soft); }
  .previous summary { display: flex; align-items: center; gap: 6px; width: fit-content; list-style: none; cursor: pointer; font-size: 12px; font-weight: 600; color: var(--text-3); }
  .previous summary::-webkit-details-marker { display: none; }
  .previous summary:hover { color: var(--text-2); }
  .previous summary :global(svg) { flex: none; transition: transform var(--dur-fast) var(--ease); }
  .previous[open] summary :global(svg) { transform: rotate(90deg); }
  .previous-body { margin-top: 12px; display: flex; flex-direction: column; gap: 12px; }
  .actions { display: flex; justify-content: flex-end; }
</style>
