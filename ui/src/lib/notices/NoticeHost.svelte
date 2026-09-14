<script lang="ts">
  import { noticesState } from "../state/notices.svelte";
  import Banner from "./Banner.svelte";
  import Toast from "./Toast.svelte";

  const state = noticesState();
</script>

<div class="notice-host" data-testid="notice-host">
  <div class="banners">
    {#each state.banners as banner (banner.localId)}
      <Banner notice={banner} />
    {/each}
  </div>
  <div class="toasts">
    {#each state.toasts as toast (toast.localId)}
      <Toast notice={toast} />
    {/each}
  </div>
</div>

<style>
  .notice-host {
    position: fixed;
    inset: 0;
    display: flex;
    flex-direction: column;
    pointer-events: none;
    z-index: var(--pv-z-toast);
  }

  .banners {
    display: flex;
    flex-direction: column;
  }

  .banners :global(.banner) {
    pointer-events: auto;
  }

  .toasts {
    margin-top: auto;
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: var(--pv-space-2);
    padding: var(--pv-space-3);
  }

  .toasts :global(.toast) {
    pointer-events: auto;
    max-width: 24rem;
  }
</style>
