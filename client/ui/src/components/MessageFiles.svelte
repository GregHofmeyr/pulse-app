<script lang="ts">
  // A message's files (#57): images inline (one large, several in a grid; click for the viewer), videos in the clip
  // player, audio in a compact player, everything else (and an image that won't load) as a file card.
  import type { Message } from '../lib/protocol/Message'
  import type { Attachment } from '../lib/protocol/Attachment'
  import { arrange, gridColumns, singleBox, type VideoSource } from '../lib/attachments'
  import { attachmentLink } from '../lib/attachmentLinks'
  import { api } from '../lib/tauri'
  import { saveFile } from '../lib/clipDownload'
  import { downloads, fileKey } from '../lib/downloads.svelte'
  import ProgressRing from './ProgressRing.svelte'
  import ClipPlayer from './ClipPlayer.svelte'
  import FileCard from './FileCard.svelte'
  import ImageViewer from './ImageViewer.svelte'
  import Icon from './Icon.svelte'

  let { message, onError }: { message: Message; onError?: (text: string) => void } = $props()

  const parts = $derived(arrange(message.attachments ?? []))
  // links by attachment id; an image whose link or bytes fail twice shows as a file card instead
  let urls = $state<Record<string, string>>({})
  let broken = $state<Record<string, boolean>>({})
  const retried = new Set<string>()
  const images = $derived(parts.images.filter((a) => !broken[a.id]))
  const cols = $derived(gridColumns(images.length))
  let viewing = $state<number | null>(null)

  const link = (a: Attachment, fresh = false) => attachmentLink(message.id, a.id, api.attachmentUrls, { fresh })

  function load(a: Attachment, fresh = false) {
    link(a, fresh).then(
      (url) => (urls[a.id] = url),
      () => (broken[a.id] = true),
    )
  }

  $effect(() => {
    for (const a of [...parts.images, ...parts.audio]) if (!urls[a.id]) load(a)
  })

  // A link that stopped working (it expired while the app slept) gets one fresh try.
  function failed(a: Attachment) {
    if (retried.has(a.id)) broken[a.id] = true
    else {
      retried.add(a.id)
      load(a, true)
    }
  }

  const save = (a: Attachment) => saveFile(message.id, a, (text) => onError?.(text))
  // the audio row's download icon (#205): a ring round a stop cross while it saves, a check once saved
  const dlLabel = (a: Attachment) => {
    const v = downloads.view(fileKey(a.id))
    if (v?.phase === 'saved') return `Saved ${a.name}`
    if (v?.phase !== 'downloading') return `Download ${a.name}`
    return v.fraction === null ? `Cancel download of ${a.name}` : `Cancel download of ${a.name}, ${Math.round(v.fraction * 100)}%`
  }

  const video = (a: Attachment): VideoSource => ({
    id: `${message.id}/${a.id}`,
    link: (fresh) => link(a, fresh),
    width: a.width,
    height: a.height,
    durationMs: a.duration_ms,
  })
</script>

{#if images.length === 1}
  {@const a = images[0]}
  {@const box = singleBox(a.width, a.height)}
  <button type="button" class="single" style:width="{box.width}px" style:aspect-ratio="{box.width} / {box.height}"
    aria-label={`Open ${a.name}`} onclick={() => (viewing = 0)}>
    {#if urls[a.id]}<img src={urls[a.id]} alt={a.name} draggable="false" decoding="async" onerror={() => failed(a)} />{/if}
  </button>
{:else if images.length > 1}
  <div class="grid" style:--cols={cols}>
    {#each images as a, i (a.id)}
      <button type="button" class="tile" aria-label={`Open ${a.name}`} onclick={() => (viewing = i)}>
        {#if urls[a.id]}<img src={urls[a.id]} alt={a.name} draggable="false" decoding="async" loading="lazy" onerror={() => failed(a)} />{/if}
      </button>
    {/each}
  </div>
{/if}
{#each parts.videos as a (a.id)}
  <div class="video"><ClipPlayer {message} source={video(a)} onDownload={() => void save(a)} downloadKey={fileKey(a.id)} /></div>
{/each}
{#each parts.audio.filter((a) => !broken[a.id]) as a (a.id)}
  {@const dl = downloads.view(fileKey(a.id))}
  <div class="audio">
    <span class="audio-name"><Icon name="music" size={14} /> <span class="audio-title">{a.name}</span>
      <button type="button" class="icon-btn icon-btn-xs dl" aria-label={dlLabel(a)} data-tip={dl?.phase === 'downloading' ? 'Cancel download' : dl?.phase === 'saved' ? 'Saved' : 'Download'} onclick={() => void save(a)}>
        {#if dl?.phase === 'downloading'}<ProgressRing fraction={dl.fraction} size={20} /><Icon name="x" size={10} />{:else}<Icon name={dl?.phase === 'saved' ? 'check' : 'download'} size={14} />{/if}
      </button></span>
    {#if urls[a.id]}<audio controls preload="metadata" src={urls[a.id]} onerror={() => failed(a)}></audio>{/if}
  </div>
{/each}
{#each [...parts.files, ...parts.images.filter((a) => broken[a.id]), ...parts.audio.filter((a) => broken[a.id])] as a (a.id)}
  <FileCard file={a} messageId={message.id} {onError} />
{/each}
{#if viewing !== null && images.length}
  <ImageViewer {images} messageId={message.id} start={Math.min(viewing, images.length - 1)} onClose={() => (viewing = null)} {onError} />
{/if}

<style>
  .single, .tile { padding: 0; border: 0; background: var(--bg-3); border-radius: var(--radius-xs); overflow: hidden; cursor: zoom-in; display: block; }
  .single { margin-top: 6px; max-width: 100%; }
  .single img, .tile img { width: 100%; height: 100%; display: block; object-fit: cover; }
  .grid { margin-top: 6px; display: grid; grid-template-columns: repeat(var(--cols), minmax(0, 1fr)); gap: 4px; max-width: 432px; }
  .tile { aspect-ratio: 1; }
  .video { margin-top: 6px; max-width: 432px; }
  .audio {
    margin-top: 6px; max-width: 432px; box-sizing: border-box; padding: 10px 12px; display: flex; flex-direction: column; gap: 8px;
    border-radius: var(--radius-xs); background: var(--bg-2); border: 1px solid var(--line-soft);
  }
  .audio-name { display: flex; align-items: center; gap: 6px; font-size: 13px; color: var(--text); min-width: 0; }
  .dl { position: relative; }
  .audio-title { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .audio audio { width: 100%; height: 32px; }
</style>
