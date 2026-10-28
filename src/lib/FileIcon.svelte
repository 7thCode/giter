<script lang="ts">
  import { extensionOf } from "./format";

  let { name, isDir, size = 20 }: { name: string; isDir: boolean; size?: number } = $props();
  const ext = $derived(extensionOf(name));
</script>

{#if isDir}
  <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden="true">
    <path
      d="M2 6.5A2.5 2.5 0 0 1 4.5 4h4.2c.5 0 .9.2 1.2.5L11.4 6H19.5A2.5 2.5 0 0 1 22 8.5v9a2.5 2.5 0 0 1-2.5 2.5h-15A2.5 2.5 0 0 1 2 17.5z"
      fill="var(--folder)"
    />
  </svg>
{:else}
  <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden="true">
    <path d="M6 2h8l5 5v13a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2z" fill="var(--file)" stroke="var(--file-edge)" />
    <path d="M14 2v5h5" fill="var(--file-edge)" />
    {#if ext}
      <text x="12" y="17" font-size="5.5" text-anchor="middle" fill="var(--text-dim)" font-family="system-ui">{ext}</text>
    {/if}
  </svg>
{/if}
