<script lang="ts">
  let { path, size = 16 }: { path: string; size?: number } = $props();
</script>

<svg
  aria-hidden="true"
  height={size}
  viewBox="0 0 24 24"
  width={size}
  xmlns="http://www.w3.org/2000/svg"
>
  <path d={path}></path>
</svg>

<style>
  svg {
    display: block;
    fill: currentColor;
  }
</style>
