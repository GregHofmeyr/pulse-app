<script lang="ts">
  // Files queued in the composer (#57): a chip each, removable, above the message box. Also the pending row's files
  // while a message with files uploads (`progress`, then no remove buttons).
  import Icon from './Icon.svelte'
  import { formatSize } from '../lib/attachments'

  type ChipFile = { id: string; name: string; size: number; kind: string; preview: string | null }

  let {
    files,
    onRemove,
    error = null,
    progress = null,
  }: {
    files: readonly ChipFile[]
    /** Absent: the chips can't be removed (a message on its way). */
    onRemove?: (id: string) => void
    /** Why the last file couldn't be added. */
    error?: string | null
    /** 0..1 while uploading. */
    progress?: number | null
  } = $props()
</script>

{#if files.length || error}
  <div class="files">
    {#if files.length}
      <ul class="chips" aria-label="Files">
        {#each files as f (f.id)}
          <li class="chip">
            {#if f.preview}
              <img class="thumb" src={f.preview} alt="" draggable="false" />
            {:else}
              <span class="thumb glyph"><Icon name={f.kind === 'video' ? 'film' : f.kind === 'audio' ? 'music' : 'file'} size={16} /></span>
            {/if}
            <span class="meta">
              <span class="name">{f.name}</span>
              <span class="size">{formatSize(f.size)}</span>
            </span>
            {#if onRemove}
              <button class="icon-btn icon-btn-xs remove" type="button" aria-label={`Remove ${f.name}`} onclick={() => onRemove(f.id)}>
                <Icon name="x" size={14} />
              </button>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
    {#if progress !== null}
      <div class="bar" role="progressbar" aria-label="Uploading" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(progress * 100)}>
        <span style:transform={`scaleX(${progress})`}></span>
      </div>
    {/if}
    {#if error}<p class="error" role="alert">{error}</p>{/if}
  </div>
{/if}

<style>
  .files { padding: 10px 12px 0; }
  .chips { list-style: none; margin: 0; padding: 0; display: flex; flex-wrap: wrap; gap: 8px; }
  .chip {
    display: flex; align-items: center; gap: 8px; max-width: 240px; padding: 6px 6px 6px 6px; box-sizing: border-box;
    border-radius: var(--radius-xs); background: var(--bg-2); border: 1px solid var(--line-soft);
  }
  .thumb { width: 36px; height: 36px; flex: none; border-radius: var(--radius-xs); object-fit: cover; background: var(--bg-3); }
  .glyph { display: grid; place-items: center; color: var(--text-3); }
  .meta { display: flex; flex-direction: column; min-width: 0; }
  .name { font-size: 13px; color: var(--text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .size { font-size: 11px; color: var(--text-3); }
  .remove { flex: none; }
  /* one step up from the chip's bg-2 */
  .remove:hover { background: var(--bg-4); }
  .bar { margin-top: 8px; height: 4px; border-radius: var(--radius-xs); background: var(--bg-4); overflow: hidden; }
  .bar span { display: block; height: 100%; background: var(--accent); transform-origin: left; transition: transform var(--dur) var(--ease); }
  .error { margin: 6px 0 0; font-size: 12px; color: var(--danger-text); }
</style>
