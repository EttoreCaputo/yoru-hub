<!-- One Untitled UI icon (icons/icons.ts), in the text's colour. -->
<script lang="ts">
  import { ICONS, type IconName } from "../icons/icons";

  let { name, size = 18, stroke = 1.75 }: { name: IconName; size?: number; stroke?: number } = $props();
</script>

<svg
  class="icon"
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width={stroke}
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true">{@html ICONS[name]}</svg>

<style>
  .icon {
    flex-shrink: 0;
    display: block;
  }
</style>
