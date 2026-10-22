<script lang="ts">
  // THROWAWAY (spike S18): the pop-out viewer. Connects as `#v2`, watches, and tells main its first frame.
  import { onMount } from 'svelte'
  import { emit } from '@tauri-apps/api/event'
  import { logResult } from './log'
  import { connect, firstStreamer, watch } from './rooms'

  let { streamer, channel }: { streamer: string; channel: string } = $props()
  let el = $state<HTMLVideoElement | undefined>()

  onMount(() => {
    void logResult('S18', async () => {
      const room = await connect(channel, 'v2', false)
      const who = streamer || firstStreamer(room)
      if (!who) throw new Error('nobody is live')
      await watch(room, who, el!)
      const firstFrameAt = Date.now()
      await emit('spike-popout-first-frame', { firstFrameAt })
      return { event: 'popout_first_frame', firstFrameAt }
    })
  })
</script>

<!-- svelte-ignore a11y_media_has_caption -->
<video bind:this={el} autoplay playsinline muted></video>

<style>
  video { width: 100vw; height: 100vh; background: #000; object-fit: contain; }
</style>
