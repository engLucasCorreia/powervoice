<script lang="ts">
  import { t } from "../i18n";
</script>

<main class="editor" data-testid="editor">
  <p>{t("panel.editor.empty")}</p>
</main>

<style>
  .editor {
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--surface-inset);
    color: var(--text-secondary);
    min-width: 0;
  }
</style>
