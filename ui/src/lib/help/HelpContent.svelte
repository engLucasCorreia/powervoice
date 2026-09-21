<script lang="ts">
  import type { HelpBlock, HelpSpan } from "./content";
  import { selectHelpSection } from "./helpCentre.svelte";

  /**
   * H-107: renders one Help Centre section's generated blocks. Deliberately not `{@html}` — the
   * generator hands over structured blocks/spans (see `content.ts`), not markup, so there's never
   * raw HTML to inject in the first place. An internal cross-reference (`span.link`, resolved at
   * generation time against the docs' own headings) is a real button that jumps the Help Centre to
   * that section, not a dead link out of the app.
   */
  let { blocks }: { blocks: HelpBlock[] } = $props();
</script>

{#snippet spans(list: HelpSpan[])}
  {#each list as span, i (i)}
    {#if span.link}
      {@const link = span.link}
      <button type="button" class="help-link" onclick={() => selectHelpSection(link.doc, link.section)}
        >{span.text}</button
      >
    {:else if span.bold}
      <strong>{span.text}</strong>
    {:else if span.italic}
      <em>{span.text}</em>
    {:else if span.code}
      <code>{span.text}</code>
    {:else}
      {span.text}
    {/if}
  {/each}
{/snippet}

<div class="help-content">
  {#each blocks as block, i (i)}
    {#if block.type === "h3"}
      <h3 id="help-anchor-{block.id}">{@render spans(block.spans)}</h3>
    {:else if block.type === "p"}
      <p>{@render spans(block.spans)}</p>
    {:else if block.type === "ul"}
      <ul>
        {#each block.items as item, j (j)}
          <li>{@render spans(item)}</li>
        {/each}
      </ul>
    {:else if block.type === "ol"}
      <ol>
        {#each block.items as item, j (j)}
          <li>{@render spans(item)}</li>
        {/each}
      </ol>
    {:else if block.type === "code"}
      <pre class="code" data-lang={block.lang}><code>{block.text}</code></pre>
    {:else if block.type === "table"}
      <div class="table-wrap">
        <table>
          <thead>
            <tr>
              {#each block.head as cell, j (j)}
                <th>{@render spans(cell)}</th>
              {/each}
            </tr>
          </thead>
          <tbody>
            {#each block.rows as row, j (j)}
              <tr>
                {#each row as cell, k (k)}
                  <td>{@render spans(cell)}</td>
                {/each}
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
  {/each}
</div>

<style>
  .help-content {
    display: flex;
    flex-direction: column;
    gap: var(--pv-space-3);
    color: var(--pv-text-primary);
    font-size: var(--pv-text-sm);
    line-height: var(--pv-leading-md);
  }

  .help-content :global(h3) {
    margin: var(--pv-space-2) 0 0;
    color: var(--pv-text-primary);
    font-size: var(--pv-text-md);
    font-weight: 600;
  }

  .help-content :global(p),
  .help-content :global(ul),
  .help-content :global(ol) {
    margin: 0;
  }

  .help-content :global(ul),
  .help-content :global(ol) {
    padding-inline-start: var(--pv-space-5);
  }

  .help-content :global(li + li) {
    margin-top: var(--pv-space-1);
  }

  .help-content :global(code) {
    padding: 0 0.25em;
    border-radius: var(--pv-radius-sm);
    background: var(--pv-bg-inset);
    font-family: var(--pv-font-mono);
    font-size: 0.9em;
  }

  .help-content :global(.code) {
    margin: 0;
    padding: var(--pv-space-3);
    overflow-x: auto;
    border: var(--pv-border-width) solid var(--pv-border);
    border-radius: var(--pv-radius-md);
    background: var(--pv-bg-inset);
    font-family: var(--pv-font-mono);
    font-size: var(--pv-text-xs);
  }

  .help-content :global(.help-link) {
    padding: 0;
    border: none;
    background: none;
    color: var(--pv-accent);
    font: inherit;
    text-decoration: underline;
    cursor: pointer;
  }

  .help-content :global(.help-link:hover) {
    text-decoration: none;
  }

  .table-wrap {
    overflow-x: auto;
  }

  .help-content :global(table) {
    border-collapse: collapse;
    width: 100%;
    font-size: var(--pv-text-sm);
  }

  .help-content :global(th),
  .help-content :global(td) {
    padding: var(--pv-space-2);
    border: var(--pv-border-width) solid var(--pv-border);
    text-align: left;
  }

  .help-content :global(th) {
    background: var(--pv-bg-inset);
    font-weight: 600;
  }
</style>
