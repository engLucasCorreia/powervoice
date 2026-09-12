<script lang="ts">
  import { t } from "../i18n";
</script>

<aside class="markers-properties" data-testid="markers-properties">
  <h2>{t("panel.markers.title")}</h2>
  <h2>{t("panel.properties.title")}</h2>
</aside>

<style>
  .markers-properties {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    background: var(--surface-panel);
    border-right: 1px solid var(--surface-border);
    padding: 0.75rem;
    overflow-y: auto;
  }

  h2 {
    font-size: 0.8rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-secondary);
    margin: 0;
  }
</style>
