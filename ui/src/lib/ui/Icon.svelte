<script lang="ts">
  import { ICONS, ICON_SIZE_PX, type IconName, type IconSize } from "./icons";

  /**
   * Lucide wrapper (H-25, A-017). Decorative by default (`aria-hidden`); pass `label` only when
   * the icon carries meaning on its own (a status glyph with no text next to it). Buttons name
   * themselves — an icon inside an IconButton stays decorative. Stroke 1.75 at 16 px matches
   * the calm, thin chrome; `filled` is for solid glyphs like the record dot.
   */
  let {
    name,
    size = "md",
    label,
    filled = false,
    testid,
  }: {
    name: IconName;
    size?: IconSize | number;
    label?: string;
    filled?: boolean;
    testid?: string;
  } = $props();

  const Glyph = $derived(ICONS[name]);
  const px = $derived(typeof size === "number" ? size : ICON_SIZE_PX[size]);
</script>

{#if label}
  <Glyph
    size={px}
    strokeWidth={1.75}
    fill={filled ? "currentColor" : "none"}
    role="img"
    aria-label={label}
    class="pv-icon"
    data-testid={testid}
  />
{:else}
  <Glyph
    size={px}
    strokeWidth={1.75}
    fill={filled ? "currentColor" : "none"}
    aria-hidden="true"
    focusable="false"
    class="pv-icon"
    data-testid={testid}
  />
{/if}
