<script lang="ts">
  // The editor's left menu (#727): one entry per section, with an icon. App appearance sits apart under a divider. It
  // never scrolls sideways; in a narrow window the labels go and only the icons stay (each still named for assistive
  // tech and by its tooltip).
  import Icon, { type IconName } from '../Icon.svelte'
  import type { EditorSection } from '../../lib/profileEditor.svelte'

  let { section, onPick }: { section: EditorSection; onPick: (s: EditorSection) => void } = $props()

  const PROFILE: { key: EditorSection; label: string; icon: IconName }[] = [
    { key: 'looks', label: 'Looks', icon: 'smile' },
    { key: 'badges', label: 'Badges', icon: 'trophy' },
    { key: 'stats', label: 'Card stats', icon: 'poll' },
    { key: 'typing', label: 'Typing text', icon: 'message' },
  ]
  const APP: { key: EditorSection; label: string; icon: IconName } = { key: 'appearance', label: 'App appearance', icon: 'gear' }
</script>

{#snippet entry(e: { key: EditorSection; label: string; icon: IconName })}
  <button type="button" class="entry" class:on={section === e.key} aria-current={section === e.key ? 'page' : undefined} aria-label={e.label} data-tip={e.label} onclick={() => onPick(e.key)}>
    <Icon name={e.icon} size={18} /><span class="label">{e.label}</span>
  </button>
{/snippet}

<nav aria-label="Profile editor sections">
  {#each PROFILE as e (e.key)}{@render entry(e)}{/each}
  <span class="sep" role="separator"></span>
  {@render entry(APP)}
</nav>

<style>
  nav { width: 200px; flex-shrink: 0; min-height: 0; overflow-y: auto; overflow-x: hidden; padding: 16px 12px; background: var(--bg-0); display: flex; flex-direction: column; gap: 2px; }
  .entry { display: flex; align-items: center; gap: 10px; min-width: 0; padding: 8px 10px; border: 0; border-radius: var(--radius-sm); background: transparent; color: var(--text-2); font-weight: 600; text-align: left; }
  .entry :global(svg) { flex-shrink: 0; }
  .label { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .entry:hover { background: var(--bg-2); color: var(--text); }
  .entry.on { background: var(--bg-3); color: var(--text); }
  .sep { height: 1px; margin: 8px 10px; background: var(--line-soft); }
  /* a narrow window: icons only, the same column, no sideways scroll */
  @media (max-width: 960px) {
    nav { width: 56px; padding: 16px 8px; align-items: stretch; }
    .entry { justify-content: center; padding: 10px 0; }
    .label { display: none; }
    .sep { margin: 8px 6px; }
  }
</style>
