<script lang="ts">
  import { t } from "../i18n";
  import VerticalMeter from "../meters/VerticalMeter.svelte";
  import { clearOutputClip, transportState } from "../state/transport.svelte";

  /**
   * The output meter (H-41 owner request, generalized into the shared `meters/VerticalMeter.svelte`
   * by H-112 so the input meter can look and read exactly like it): vertical, fixed-width,
   * readable, with a click-to-clear clip lamp. See `VerticalMeter.svelte` for the shared bar/scale
   * implementation.
   */

  const transport = transportState();
</script>

<VerticalMeter
  label={t("meter.output")}
  testid="output-meter"
  peakDbfs={transport.meter.peakDbfs}
  rmsDbfs={transport.meter.rmsDbfs}
  holdDbfs={transport.meter.holdDbfs}
  clip={transport.meter.clip}
  peakReadoutDbfs={transport.meter.peakReadoutDbfs}
  rmsReadoutDbfs={transport.meter.rmsReadoutDbfs}
  clipLamp={{ label: t("meter.clip"), title: t("meter.clip_title"), onclick: clearOutputClip }}
/>
