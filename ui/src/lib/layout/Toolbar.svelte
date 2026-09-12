<script lang="ts">
  import { t } from "../i18n";

  let { version = "" }: { version?: string } = $props();
</script>

<header class="toolbar" data-testid="toolbar">
  <span class="app-name">{t("app.title")}</span>
  <span class="version" data-testid="app-version">{version}</span>
  <div class="transport">
    <button type="button">{t("toolbar.transport.play")}</button>
  </div>
</header>

<style>
  .toolbar {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.5rem 0.75rem;
    background: var(--surface-panel);
    border-bottom: 1px solid var(--surface-border);
  }

  .app-name {
    font-weight: 600;
  }

  .version {
    color: var(--text-secondary);
    font-variant-numeric: tabular-nums;
  }

  .transport {
    margin-left: auto;
  }

  button {
    background: var(--surface-panel-raised);
    color: var(--text-primary);
    border: 1px solid var(--surface-border);
    border-radius: 4px;
    padding: 0.25rem 0.75rem;
  }

  button:hover {
    border-color: var(--accent);
  }
</style>
