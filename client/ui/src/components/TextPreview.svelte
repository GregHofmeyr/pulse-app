<script lang="ts">
  // A small text file's inline preview (#318), under its file card: the first lines, expandable to the whole file in a
  // scrollable box. Markdown renders through the app's safe renderer; everything else, HTML included, is shown as its
  // source text (a text node in a <pre>, never markup). A file that isn't UTF-8 (or is binary) says it can't be shown.
  import { renderMarkdown } from '../lib/markdown'
  import { firstLines } from '../lib/textPreview'
  import { api, errorText } from '../lib/tauri'
  import type { Attachment } from '../lib/protocol/Attachment'

  let { file, messageId, kind }: { file: Attachment; messageId: string; kind: 'markdown' | 'text' } = $props()

  let text = $state<string | null>(null)
  let failed = $state('')
  let expanded = $state(false)
  $effect(() => {
    const id = file.id
    const msg = messageId
    text = null
    failed = ''
    api.attachmentText(msg, id).then(
      (t) => {
        if (id !== file.id) return
        if (typeof t !== 'string') failed = "Can't preview this file."
        else text = t
      },
      (e) => {
        if (id === file.id) failed = `Couldn't load the preview: ${errorText(e)}`
      },
    )
  })
  const head = $derived(text === null ? null : firstLines(text))
  const shown = $derived(text === null || !head ? '' : expanded ? text : head.head)
  // renderMarkdown's output is DOMPurify-sanitised (the same allow-list as messages): the only HTML this renders
  const html = $derived(kind === 'markdown' && text !== null ? renderMarkdown(shown) : '')
</script>

<div class="preview" class:expanded data-text-preview={file.id}>
  {#if failed}
    <p class="note">{failed}</p>
  {:else if text === null}
    <p class="note">Loading preview…</p>
  {:else}
    <!-- svelte-ignore a11y_no_noninteractive_tabindex (the whole file scrolls: reachable by keyboard to scroll it) -->
    <div class="body" class:scroll={expanded} tabindex={expanded ? 0 : undefined} role={expanded ? 'region' : undefined} aria-label={expanded ? `${file.name}, whole file` : undefined}>
      {#if kind === 'markdown'}
        <div class="md">{@html html}</div>
      {:else}
        <pre>{shown}</pre>
      {/if}
    </div>
    {#if head?.more}
      <button type="button" class="link-btn toggle" aria-expanded={expanded} onclick={() => (expanded = !expanded)}>
        {expanded ? 'Show less' : `Show all ${head.lines} lines`}
      </button>
    {/if}
  {/if}
</div>

<style>
  .preview { margin-top: 4px; max-width: 432px; box-sizing: border-box; padding: 8px 12px; border-radius: var(--radius-xs); background: var(--bg-inset); border: 1px solid var(--line-soft); }
  .body { min-width: 0; }
  .body.scroll { max-height: 360px; overflow: auto; }
  pre { margin: 0; font-family: 'JetBrains Mono', ui-monospace, monospace; font-size: 12px; line-height: 1.5; color: var(--text); white-space: pre-wrap; overflow-wrap: anywhere; }
  .md { font-size: 13px; line-height: 1.45; color: var(--text); overflow-wrap: anywhere; }
  .md :global(:first-child) { margin-top: 0; }
  .md :global(:last-child) { margin-bottom: 0; }
  .md :global(p), .md :global(ul), .md :global(ol), .md :global(blockquote) { margin: 4px 0; }
  .md :global(code) { font-family: 'JetBrains Mono', ui-monospace, monospace; font-size: 12px; background: var(--bg-2); border-radius: var(--radius-xs); padding: 1px 4px; }
  .md :global(pre) { margin: 4px 0; padding: 8px; overflow-x: auto; background: var(--bg-2); border-radius: var(--radius-xs); }
  .md :global(pre code) { padding: 0; background: none; }
  .note { margin: 0; font-size: 12px; color: var(--text-3); }
  .toggle { margin-top: 6px; font-size: 12px; }
</style>
