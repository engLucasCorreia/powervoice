<script lang="ts">
  import { t } from "../i18n";
  import { Dialog, EmptyState, Icon, IconButton } from "../ui";
  import { HELP_DOCS, searchHelp, type HelpSearchHit } from "./content";
  import HelpContent from "./HelpContent.svelte";
  import { closeHelpCentre, helpCentreState, selectHelpSection, setHelpQuery } from "./helpCentre.svelte";

  /**
   * H-107: a browsable, searchable, offline Help Centre — reachable from Help ▸ Help Centre…, F1
   * (`shortcuts/registry.ts`), and a "?" in the panels that need it most (`HelpButton.svelte`). Its
   * content (`content.ts`/`content.generated.ts`) is generated from `docs/user-guide.md` and
   * `docs/faq.md` (`scripts/help/generate.py`) — never a website link, since the whole point is
   * helping someone who can't get audio working and may not be online.
   *
   * Layout: a nav of every doc's sections on the left, the selected section's content on the
   * right; typing in the search box replaces both with a flat list of matching sections (title and
   * body text both searched, see `content.ts::searchHelp`).
   */
  const hs = helpCentreState();

  const results = $derived(searchHelp(hs.query));
  const searching = $derived(hs.query.trim().length > 0);
  const currentDoc = $derived(HELP_DOCS.find((d) => d.id === hs.docId));
  const currentSection = $derived(currentDoc?.sections.find((s) => s.id === hs.sectionId));

  function onKeydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === "Escape") {
      if (hs.query) {
        setHelpQuery("");
        return;
      }
      closeHelpCentre();
    }
  }

  function onSearchKeydown(event: KeyboardEvent): void {
    // Typing in the search field must never reach the editor's shortcuts (same rule as the Plugin
    // Manager's own search field) — only claim Escape, and only to clear a non-empty query; a
    // second Escape (query already empty) falls through to the Dialog's own handler and closes it.
    if (event.key === "Escape" && hs.query) {
      event.stopPropagation();
      setHelpQuery("");
    }
  }

  function openHit(hit: HelpSearchHit): void {
    selectHelpSection(hit.docId, hit.sectionId);
    setHelpQuery("");
  }
</script>

{#if hs.open}
  <Dialog
    size="xl"
    title={t("help.title")}
    titleId="help-centre-title"
    testid="help-centre"
    onkeydown={onKeydown}
    actions={[{ label: t("help.close"), role: "primary", testid: "help-centre-close", onclick: closeHelpCentre }]}
  >
    <div class="help">
      <label class="search">
        <Icon name="search" size="sm" />
        <input
          type="search"
          placeholder={t("help.search")}
          aria-label={t("help.search")}
          data-testid="help-search"
          value={hs.query}
          oninput={(e) => setHelpQuery(e.currentTarget.value)}
          onkeydown={onSearchKeydown}
        />
        {#if hs.query}
          <IconButton
            icon="close"
            size="sm"
            label={t("help.search_clear")}
            testid="help-search-clear"
            onclick={() => setHelpQuery("")}
          />
        {/if}
      </label>

      <div class="body">
        {#if searching}
          <div class="results" data-testid="help-results">
            {#if results.length === 0}
              <EmptyState icon="search" title={t("help.no_results")} description={t("help.no_results_hint")} size="sm" />
            {:else}
              <p class="results-count" data-testid="help-results-count">
                {results.length === 1
                  ? t("help.search_results_one", { query: hs.query })
                  : t("help.search_results", { count: results.length, query: hs.query })}
              </p>
              <ul class="result-list">
                {#each results as hit (hit.docId + ":" + hit.sectionId)}
                  <li>
                    <button
                      type="button"
                      class="result"
                      data-testid="help-result-{hit.docId}-{hit.sectionId}"
                      onclick={() => openHit(hit)}
                    >
                      <span class="result-title">{hit.sectionTitle}</span>
                      <span class="result-doc">{hit.docTitle}</span>
                      <span class="result-snippet">{hit.snippet}</span>
                    </button>
                  </li>
                {/each}
              </ul>
            {/if}
          </div>
        {:else}
          <nav class="nav" aria-label={t("help.nav_label")} data-testid="help-nav">
            {#each HELP_DOCS as doc (doc.id)}
              <h3 class="nav-doc">{doc.title}</h3>
              <ul>
                {#each doc.sections as section (section.id)}
                  <li>
                    <button
                      type="button"
                      class="nav-item"
                      aria-current={hs.docId === doc.id && hs.sectionId === section.id ? "page" : undefined}
                      data-testid="help-nav-{doc.id}-{section.id}"
                      onclick={() => selectHelpSection(doc.id, section.id)}
                    >
                      {section.title}
                    </button>
                  </li>
                {/each}
              </ul>
            {/each}
          </nav>
          <div class="content" data-testid="help-content">
            {#if currentSection}
              <h2 class="content-title">{currentSection.title}</h2>
              <HelpContent blocks={currentSection.blocks} />
            {/if}
          </div>
        {/if}
      </div>
    </div>
  </Dialog>
{/if}

<style>
  .help {
    display: flex;
    flex-direction: column;
    gap: var(--pv-space-3);
    height: min(680px, 78vh);
    min-height: 0;
  }

  .search {
    display: flex;
    flex: none;
    align-items: center;
    gap: var(--pv-space-2);
    max-width: 26rem;
    height: var(--pv-control-h-md);
    padding-inline: var(--pv-control-px-sm) var(--pv-space-1);
    border: var(--pv-border-width) solid var(--pv-border-control);
    border-radius: var(--pv-radius-md);
    background: var(--pv-field-bg);
    color: var(--pv-text-tertiary);
  }

  .search:focus-within {
    border-color: var(--pv-accent);
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
    outline-offset: 0;
  }

  .search input {
    flex: 1;
    min-width: 0;
    height: 100%;
    padding: 0;
    border: none;
    background: transparent;
    color: var(--pv-text-primary);
    font-family: inherit;
    font-size: var(--pv-text-md);
    outline: none;
  }

  .search input:focus-visible {
    border: none;
    outline: none;
  }

  .search input::-webkit-search-cancel-button {
    display: none;
  }

  .search input::placeholder {
    color: var(--pv-text-tertiary);
  }

  .body {
    display: flex;
    flex: 1;
    gap: var(--pv-space-4);
    min-height: 0;
  }

  .nav {
    flex: none;
    width: 13rem;
    overflow-y: auto;
    border-right: var(--pv-border-width) solid var(--pv-border);
    padding-right: var(--pv-space-3);
  }

  /* Phone-width windows (the Dialog itself shrinks to 90vw below ~980px, `xl`'s own 880px): a
   * side-by-side nav squeezes the content column illegibly narrow, so stack nav above content
   * instead — same content, no cramped two-column layout. */
  @media (max-width: 640px) {
    .body {
      flex-direction: column;
    }

    .nav {
      flex: none;
      width: 100%;
      max-height: 40%;
      border-right: none;
      border-bottom: var(--pv-border-width) solid var(--pv-border);
      padding-right: 0;
      padding-bottom: var(--pv-space-2);
    }
  }

  .nav-doc {
    margin: var(--pv-space-3) 0 var(--pv-space-1);
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-xs);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .nav-doc:first-child {
    margin-top: 0;
  }

  .nav ul {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .nav-item {
    width: 100%;
    padding: var(--pv-space-1) var(--pv-space-2);
    border: none;
    border-radius: var(--pv-radius-sm);
    background: none;
    color: var(--pv-text-secondary);
    font: inherit;
    font-size: var(--pv-text-sm);
    text-align: left;
    cursor: pointer;
  }

  .nav-item:hover {
    background: var(--pv-bg-inset);
    color: var(--pv-text-primary);
  }

  .nav-item[aria-current="page"] {
    background: var(--pv-accent-soft);
    color: var(--pv-accent-text);
    font-weight: 600;
  }

  .content {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
  }

  .content-title {
    margin: 0 0 var(--pv-space-3);
    color: var(--pv-text-primary);
    font-size: var(--pv-text-lg);
  }

  .results {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
  }

  .results-count {
    margin: 0 0 var(--pv-space-2);
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-sm);
  }

  .result-list {
    display: flex;
    flex-direction: column;
    gap: var(--pv-space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .result {
    display: flex;
    flex-direction: column;
    gap: var(--pv-space-1);
    width: 100%;
    padding: var(--pv-space-2) var(--pv-space-3);
    border: var(--pv-border-width) solid var(--pv-border);
    border-radius: var(--pv-radius-md);
    background: var(--pv-bg-raised);
    color: var(--pv-text-primary);
    text-align: left;
    cursor: pointer;
  }

  .result:hover {
    border-color: var(--pv-border-strong);
    background: var(--pv-bg-inset);
  }

  .result-title {
    font-size: var(--pv-text-md);
    font-weight: 600;
  }

  .result-doc {
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-xs);
  }

  .result-snippet {
    color: var(--pv-text-secondary);
    font-size: var(--pv-text-sm);
  }
</style>
