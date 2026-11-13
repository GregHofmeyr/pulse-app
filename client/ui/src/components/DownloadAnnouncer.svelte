<script lang="ts">
  // One polite live region for every download (#205): "Downloading clip", "Saved clip", "Stopped downloading …".
  // Always in the page (a region added along with its text often isn't announced); never each percent.
  import { downloads } from '../lib/downloads.svelte'
</script>

<p class="sr-only" aria-live="polite" data-downloads>{downloads.announcement}</p>
