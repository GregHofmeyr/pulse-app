<script lang="ts">
  // For DateField.test.ts: a DateField bound to a value, inside a "form" with its own keydown handler.
  import DateField from './DateField.svelte'

  let { value: start, min = null, onChange, onFormKey, onkeydown }: {
    value: string
    min?: string | null
    onChange: (v: string) => void
    onFormKey?: (e: KeyboardEvent) => void
    onkeydown?: (e: KeyboardEvent) => void
  } = $props()
  // svelte-ignore state_referenced_locally
  let value = $state(start)
  $effect(() => {
    if (value !== start) onChange(value)
  })
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="form" onkeydown={(e) => onFormKey?.(e)}>
  <DateField bind:value label="Day" {min} {onkeydown} />
</div>
