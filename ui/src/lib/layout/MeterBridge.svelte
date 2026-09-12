<script lang="ts">
  import { t } from "../i18n";
</script>

<footer class="meter-bridge" data-testid="meter-bridge">
  <span>{t("panel.meters.title")}</span>
</footer>

<style>
  .meter-bridge {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.4rem 0.75rem;
    background: var(--surface-panel);
    border-top: 1px solid var(--surface-border);
    color: var(--text-secondary);
    font-size: 0.75rem;
  }
</style>
