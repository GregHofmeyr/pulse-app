<script lang="ts">
  // A message's files (#57): images inline (one large, several in a grid; click for the viewer), videos in the clip
  // player, audio in a compact player, everything else (and an image that won't load) as a file card, a small text file with its preview under it (#318). A file the sender
  // marked as a spoiler (#289) shows blurred, and can't be used, until it's clicked; revealed stays revealed here.
  // View-once files (#311) are ViewOnceFiles' for anyone but their sender.
  import type { Message } from '../lib/protocol/Message'
  import type { Attachment } from '../lib/protocol/Attachment'
  import { arrange, gridColumns, singleBox, type VideoSource } from '../lib/attachments'
  import { attachmentLink } from '../lib/attachmentLinks'
  import { api } from '../lib/tauri'
  import { saveFile } from '../lib/clipDownload'
  import { downloads, fileKey, holdSaved } from '../lib/downloads.svelte'
  import ProgressRing from './ProgressRing.svelte'
  import ClipPlayer from './ClipPlayer.svelte'
  import FileCard from './FileCard.svelte'
  import TextPreview from './TextPreview.svelte'
  import { previewKind } from '../lib/textPreview'
  import { app, clock } from '../lib/store.svelte'
  import ViewOnceFiles from './ViewOnceFiles.svelte'
  import ImageViewer from './ImageViewer.svelte'
  import Icon from './Icon.svelte'

  let { message, onError }: { message: Message; onError?: (text: string) => void } = $props()

  // view-once files (#311): their sender sees them as usual (with who opened them); anyone else gets ViewOnceFiles'
  // card instead, and none of their links are fetched here (the server would give none anyway)
  const mine = $derived(!!message.author_id && message.author_id === app.state.me?.id)
  const once = $derived((message.attachments ?? []).filter((a) => a.view_once))
  const parts = $derived(arrange((message.attachments ?? []).filter((a) => mine || !a.view_once)))
  // links by attachment id; an image whose link or bytes fail twice shows as a file card instead
  let urls = $state<Record<string, string>>({})
  let broken = $state<Record<string, boolean>>({})
  const retried = new Set<string>()
  const images = $derived(parts.images.filter((a) => !broken[a.id]))
  const cols = $derived(gridColumns(images.length))
  let viewing = $state<number | null>(null)
  // spoilers clicked open, by attachment id
  let revealed = $state<Record<string, boolean>>({})
  const covered = (a: Attachment) => !!a.spoiler && !revealed[a.id]
  // the viewer only ever pages through images you can already see
  const viewable = $derived(images.filter((a) => !covered(a)))
  function openImage(a: Attachment) {
    if (covered(a)) revealed[a.id] = true
    else viewing = viewable.indexOf(a)
  }
  const imageLabel = (a: Attachment) => (covered(a) ? `Show spoiler: ${a.name}` : `Open ${a.name}`)

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
  const reveal = (a: Attachment) => downloads.reveal(fileKey(a.id), (text) => onError?.(text))
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
  <button type="button" class="single" class:covered={covered(a)} style:width="{box.width}px" style:aspect-ratio="{box.width} / {box.height}"
    aria-label={imageLabel(a)} onclick={() => openImage(a)}>
    {#if urls[a.id]}<img src={urls[a.id]} alt={covered(a) ? '' : a.name} draggable="false" decoding="async" onerror={() => failed(a)} />{/if}
    {#if covered(a)}{@render tag()}{/if}
  </button>
{:else if images.length > 1}
  <div class="grid" style:--cols={cols}>
    {#each images as a (a.id)}
      <button type="button" class="tile" class:covered={covered(a)} aria-label={imageLabel(a)} onclick={() => openImage(a)}>
        {#if urls[a.id]}<img src={urls[a.id]} alt={covered(a) ? '' : a.name} draggable="false" decoding="async" loading="lazy" onerror={() => failed(a)} />{/if}
        {#if covered(a)}{@render tag()}{/if}
      </button>
    {/each}
  </div>
{/if}
{#each parts.videos as a (a.id)}
  <div class="video spoiler-box">
    <div class="spoiler-content" class:blurred={covered(a)} inert={covered(a)}><ClipPlayer {message} source={video(a)} onDownload={() => void save(a)} downloadKey={fileKey(a.id)} onDownloadError={onError} /></div>
    {#if covered(a)}{@render cover(a)}{/if}
  </div>
{/each}
{#each parts.audio.filter((a) => !broken[a.id]) as a (a.id)}
  {@const dl = downloads.view(fileKey(a.id))}
  <div class="spoiler-box audio-box">
  <div class="audio spoiler-content" class:blurred={covered(a)} inert={covered(a)}>
    <span class="audio-name"><Icon name="music" size={14} /> <span class="audio-title">{a.name}</span>
      <button type="button" class="icon-btn icon-btn-xs dl" aria-label={dlLabel(a)} data-tip={dl?.phase === 'downloading' ? 'Cancel download' : dl?.phase === 'saved' ? 'Saved' : 'Download'} onclick={() => void save(a)}>
        {#if dl?.phase === 'downloading'}<ProgressRing fraction={dl.fraction} size={20} /><Icon name="x" size={10} />{:else}<Icon name={dl?.phase === 'saved' ? 'check' : 'download'} size={14} />{/if}
      </button>
      {#if dl?.phase === 'saved'}<button type="button" class="folder" use:holdSaved={fileKey(a.id)} onclick={() => void reveal(a)}>Show in folder</button>{/if}</span>
    {#if urls[a.id]}<audio controls preload="metadata" src={urls[a.id]} onerror={() => failed(a)}></audio>{/if}
  </div>
  {#if covered(a)}{@render cover(a)}{/if}
  </div>
{/each}
{#each [...parts.files, ...parts.images.filter((a) => broken[a.id]), ...parts.audio.filter((a) => broken[a.id])] as a (a.id)}
  {@const preview = previewKind(a, Date.parse(clock.nowIso))}
  <div class="spoiler-box file-box">
    <div class="spoiler-content" class:blurred={covered(a)} inert={covered(a)}>
      <FileCard file={a} messageId={message.id} {onError} />
      <!-- a small text file's preview (#318): read only once a spoiler is shown -->
      {#if preview && !covered(a)}<TextPreview file={a} messageId={message.id} kind={preview} />{/if}
    </div>
    {#if covered(a)}{@render cover(a)}{/if}
  </div>
{/each}
{#if once.length}<ViewOnceFiles {message} files={once} {mine} {onError} />{/if}
{#if viewing !== null && viewable.length}
  <ImageViewer images={viewable} messageId={message.id} start={Math.min(Math.max(viewing, 0), viewable.length - 1)} onClose={() => (viewing = null)} {onError} />
{/if}

{#snippet tag()}
  <span class="spoiler-tag"><Icon name="eye" size={14} /> Spoiler</span>
{/snippet}

{#snippet cover(a: Attachment)}
  <button type="button" class="spoiler-cover" aria-label={`Show spoiler: ${a.name}`} onclick={() => (revealed[a.id] = true)}>{@render tag()}</button>
{/snippet}

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
  .folder { flex: none; padding: 0; border: 0; background: none; font-size: 12px; color: var(--text-3); cursor: pointer; }
  .folder:hover, .folder:focus-visible { color: var(--text); text-decoration: underline; }
  .audio-title { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .audio audio { width: 100%; height: 32px; }
  /* spoilers (#289): blurred and out of reach under a cover until clicked */
  .single, .tile { position: relative; }
  .covered { cursor: pointer; }
  .covered img, .blurred { filter: blur(44px); }
  .covered img { transform: scale(1.1); }
  .spoiler-box { position: relative; max-width: 432px; }
  .spoiler-box.file-box, .spoiler-box.audio-box { margin-top: 6px; }
  .file-box :global(.card), .audio-box .audio { margin-top: 0; }
  .spoiler-box:has(.blurred) { border-radius: var(--radius-xs); overflow: hidden; }
  .spoiler-cover {
    position: absolute; inset: 0; padding: 0; border: 0; border-radius: var(--radius-xs); cursor: pointer;
    display: grid; place-items: center; background: var(--scrim);
  }
  .spoiler-tag {
    position: absolute; inset: 0; margin: auto; width: max-content; height: max-content;
    display: inline-flex; align-items: center; gap: 6px; padding: 6px 12px; border-radius: 999px;
    font-size: 12px; font-weight: 600; letter-spacing: .04em; text-transform: uppercase; color: var(--tooltip-text);
    background: var(--scrim-strong);
  }
  .spoiler-cover:hover .spoiler-tag, .spoiler-cover:focus-visible .spoiler-tag,
  .covered:hover .spoiler-tag, .covered:focus-visible .spoiler-tag { background: var(--tooltip-bg); }
  @media (prefers-reduced-motion: no-preference) {
    .single img, .tile img, .spoiler-content { transition: filter var(--dur) var(--ease); }
  }
</style>
