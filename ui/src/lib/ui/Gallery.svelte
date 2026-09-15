<script lang="ts">
  import { t } from "../i18n";
  import { shortcutLabelForAction } from "../keymap/shortcutLabel";
  import Badge from "./Badge.svelte";
  import Button from "./Button.svelte";
  import EmptyState from "./EmptyState.svelte";
  import Icon from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";
  import { ICON_NAMES } from "./icons";
  import Kbd from "./Kbd.svelte";
  import Menu from "./Menu.svelte";
  import type { MenuEntry } from "./menuModel";
  import { orderDialogActions, type DialogActionRole } from "./dialogActions";
  import Popover from "./Popover.svelte";
  import NumberField from "./NumberField.svelte";
  import PanelHeader from "./PanelHeader.svelte";
  import Readout from "./Readout.svelte";
  import SegmentedControl from "./SegmentedControl.svelte";
  import Select from "./Select.svelte";
  import Separator from "./Separator.svelte";
  import Slider from "./Slider.svelte";
  import StatusDot from "./StatusDot.svelte";
  import Tabs from "./Tabs.svelte";
  import Toggle from "./Toggle.svelte";
  import ToggleButton from "./ToggleButton.svelte";
  import type { SegmentOption, SelectOption, TabItem } from "./types";
  import { formatWithUnit } from "./units";

  /**
   * Component gallery (H-25): every kit component in every state, dark and light side by side.
   * Development builds only — `main.ts` mounts it for `?gallery` when `import.meta.env.DEV`
   * (`npm --prefix ui run dev`, then open http://localhost:1420/?gallery). No IPC, no canvas.
   * Both columns share the demo state, so a change in one shows in the other.
   */
  type Response = "fast" | "medium" | "slow";
  type Dock = "meters" | "analyzer" | "loudness";
  type Source = "processed" | "source";

  const THEMES = ["dark", "light"] as const;

  const responses: SegmentOption<Response>[] = [
    { value: "fast", label: t("analyzer.response.fast") },
    { value: "medium", label: t("analyzer.response.medium") },
    { value: "slow", label: t("analyzer.response.slow") },
  ];
  const responsesWithDisabled: SegmentOption<Response>[] = [
    { value: "fast", label: t("analyzer.response.fast") },
    { value: "medium", label: t("analyzer.response.medium"), disabled: true },
    { value: "slow", label: t("analyzer.response.slow") },
  ];
  const sources: SegmentOption<Source>[] = [
    { value: "processed", label: t("gallery.sample.processed") },
    { value: "source", label: t("gallery.sample.original") },
  ];
  const floors: SelectOption<number>[] = [-120, -96, -72, -48].map((v) => ({
    value: v,
    label: formatWithUnit(v, "dB", 0),
  }));
  const dockTabs: TabItem<Dock>[] = [
    { id: "meters", label: t("gallery.sample.meters"), icon: "meters" },
    { id: "analyzer", label: t("gallery.sample.analyzer"), icon: "analyzer" },
    { id: "loudness", label: t("gallery.sample.loudness"), icon: "loudness", badge: "1" },
  ];

  const SURFACES = ["--pv-bg-inset", "--pv-bg-app", "--pv-bg-panel", "--pv-bg-raised", "--pv-bg-overlay"];
  const FILLS = [
    "--pv-control-bg",
    "--pv-control-bg-selected",
    "--pv-accent",
    "--pv-accent-fill",
    "--pv-accent-soft",
    "--pv-record",
    "--pv-record-fill",
    "--pv-danger-fill",
    "--pv-warning",
    "--pv-success",
    "--pv-meter-safe",
    "--pv-meter-caution",
    "--pv-meter-over",
    "--pv-playhead",
    "--pv-waveform",
  ];
  const TYPE_SCALE = ["xs", "sm", "md", "lg", "xl"] as const;

  const sc = {
    play: shortcutLabelForAction("transport.play_pause"),
    fromStart: shortcutLabelForAction("transport.return_to_start"),
    record: shortcutLabelForAction("record.toggle"),
    open: shortcutLabelForAction("file.open"),
    marker: shortcutLabelForAction("marker.add"),
    spectral: shortcutLabelForAction("spectral.toggle"),
    undo: shortcutLabelForAction("history.undo"),
  };

  let response = $state<Response>("medium");
  let source = $state<Source>("processed");
  let floor = $state(-96);
  let peakHold = $state(true);
  let hearOriginal = $state(false);
  let abOn = $state(false);
  let spectral = $state(true);
  let gain = $state(-3);
  let threshold = $state(-24);
  let target = $state(-1);
  let preroll = $state(2);
  let dock = $state<Dock>("analyzer");
  let rackOpen = $state(true);
  let recording = $state(false);
  let playing = $state(false);
  let loop = $state(false);

  // H-26: the shared menu, a popover and the platform dialog button order.
  let menuFor = $state<string | null>(null);
  let popoverFor = $state<string | null>(null);
  const menuAnchors: Record<string, HTMLButtonElement | undefined> = $state({});
  const popoverAnchors: Record<string, HTMLButtonElement | undefined> = $state({});
  let renderer = $state<"auto" | "webgl2">("auto");
  const noop = (): void => {};
  const demoMenu = $derived<MenuEntry[]>([
    { kind: "item", id: "open", label: t("menu.file.open"), shortcut: sc.open, onselect: noop },
    { kind: "item", id: "save", label: t("menu.file.save"), shortcut: shortcutLabelForAction("file.save"), disabled: true, onselect: noop },
    { kind: "separator", id: "sep-view" },
    { kind: "checkbox", id: "spectral", label: t("spectral.toggle"), checked: spectral, shortcut: sc.spectral, onselect: () => (spectral = !spectral) },
    {
      kind: "submenu",
      id: "renderer",
      label: t("menu.view.renderer"),
      items: [
        { kind: "radio", id: "auto", label: t("menu.view.renderer_auto"), checked: renderer === "auto", onselect: () => (renderer = "auto") },
        { kind: "radio", id: "webgl2", label: t("menu.view.renderer_webgl2"), checked: renderer === "webgl2", onselect: () => (renderer = "webgl2") },
      ],
    },
    { kind: "separator", id: "sep-normalize" },
    { kind: "heading", id: "peak", label: t("toolbar.normalize.peak_heading") },
    { kind: "item", id: "n1", label: formatWithUnit(-1, "dBFS", 1), onselect: noop },
    { kind: "item", id: "n3", label: formatWithUnit(-3, "dBFS", 1), onselect: noop },
  ]);
  const FOOTER_PLATFORMS = ["linux", "windows"] as const;
  const footerDemo: { label: string; role: DialogActionRole }[] = [
    { label: t("dialog.unsaved.discard"), role: "destructive" },
    { label: t("dialog.unsaved.cancel"), role: "cancel" },
    { label: t("dialog.unsaved.save"), role: "primary" },
  ];
</script>

<div class="gallery">
  <header class="intro">
    <h1>{t("gallery.title")}</h1>
    <p>{t("gallery.subtitle")}</p>
    <p class="hint">{t("gallery.hint.states")}</p>
  </header>

  <div class="columns">
    {#each THEMES as theme (theme)}
      <div class="column" data-theme={theme} data-testid="gallery-{theme}">
        <h2 class="column-title">{theme === "dark" ? t("gallery.theme.dark") : t("gallery.theme.light")}</h2>

        <section class="card">
          <h3>{t("gallery.section.transport")}</h3>
          <div class="transport-demo" class:on-air={recording} role="toolbar" aria-label={t("transport.group")}>
            <div class="group">
              <IconButton icon="returnToStart" label={t("transport.return_to_start")} shortcut={sc.fromStart} />
              <IconButton
                icon={playing ? "pause" : "play"}
                label={playing ? t("transport.pause") : t("transport.play")}
                shortcut={sc.play}
                size="lg"
                onclick={() => (playing = !playing)}
              />
              <IconButton icon="stop" label={t("transport.stop")} disabled={!playing} onclick={() => (playing = false)} />
              <IconButton icon="loop" label={t("gallery.sample.loop")} pressed={loop} onclick={() => (loop = !loop)} />
            </div>
            <Separator orientation="vertical" />
            <Readout text="0:01:23.456" size="xl" />
            <Separator orientation="vertical" />
            <div class="group">
              <IconButton
                icon={recording ? "stop" : "record"}
                variant="record"
                active={recording}
                label={recording ? t("record.stop") : t("record.record")}
                shortcut={sc.record}
                size="lg"
                onclick={() => (recording = !recording)}
              />
              <Readout text={recording ? "0:00:12.3" : "0:00:00.0"} tone={recording ? "record" : "muted"} />
              {#if recording}
                <StatusDot tone="record" label={t("gallery.state.recording")} showLabel pulse />
              {:else}
                <Badge>{t("gallery.sample.remaining")}</Badge>
              {/if}
            </div>
            <Separator orientation="vertical" />
            <ToggleButton bind:pressed={spectral} icon="spectral" size="sm">{t("spectral.toggle")}</ToggleButton>
            <span class="spacer"></span>
            <IconButton icon="settings" label={t("devices.open")} />
          </div>
        </section>

        <section class="card">
          <h3>{t("gallery.section.buttons")}</h3>
          <div class="row">
            <Button variant="primary">{t("gallery.sample.apply")}</Button>
            <Button>{t("gallery.sample.cancel")}</Button>
            <Button variant="ghost">{t("gallery.sample.ghost")}</Button>
            <Button variant="danger" icon="delete">{t("gallery.sample.delete")}</Button>
          </div>
          <div class="row">
            <Button variant="primary" disabled>{t("gallery.state.disabled")}</Button>
            <Button disabled>{t("gallery.state.disabled")}</Button>
            <Button variant="ghost" disabled>{t("gallery.state.disabled")}</Button>
            <Button loading>{t("gallery.sample.analyze")}</Button>
          </div>
          <div class="row">
            <Button size="sm">{t("gallery.sample.small")}</Button>
            <Button icon="open" iconEnd="chevronDown">{t("empty.document.open")}</Button>
            <Button size="lg" variant="primary" icon="record">{t("empty.document.record")}</Button>
          </div>
        </section>

        <section class="card">
          <h3>{t("gallery.section.icon_buttons")}</h3>
          <div class="row">
            <IconButton icon="play" label={t("transport.play")} shortcut={sc.play} />
            <IconButton icon="settings" label={t("devices.open")} variant="secondary" />
            <IconButton icon="add" label={t("markers.panel.add")} variant="primary" />
            <IconButton icon="loop" label={t("gallery.sample.loop")} pressed={true} />
            <IconButton icon="record" label={t("record.record")} variant="record" />
            <IconButton icon="stop" label={t("record.stop")} variant="record" active />
            <IconButton icon="delete" label={t("markers.panel.delete")} disabled />
          </div>
          <div class="row">
            <IconButton icon="more" label={t("gallery.sample.more")} size="sm" />
            <IconButton icon="more" label={t("gallery.sample.more")} />
            <IconButton icon="more" label={t("gallery.sample.more")} size="lg" />
            <IconButton icon="undo" label={t("gallery.sample.more")} shortcut={sc.undo} tooltipPlacement="top" />
          </div>
        </section>

        <section class="card">
          <h3>{t("gallery.section.choices")}</h3>
          <div class="row">
            <ToggleButton bind:pressed={abOn}>{t("rack.ab")}</ToggleButton>
            <ToggleButton pressed={true} icon="spectral">{t("spectral.toggle")}</ToggleButton>
            <ToggleButton disabled>{t("gallery.state.disabled")}</ToggleButton>
          </div>
          <div class="row">
            <SegmentedControl options={responses} bind:value={response} label={t("panel.analyzer.title")} />
            <SegmentedControl options={sources} bind:value={source} label={t("gallery.sample.source")} size="sm" />
            <SegmentedControl options={responsesWithDisabled} value="fast" label={t("panel.analyzer.title")} size="sm" />
          </div>
          <div class="stack">
            <Toggle bind:checked={peakHold} label={t("analyzer.peak_hold")} />
            <Toggle bind:checked={hearOriginal} label={t("record.hear_original")} description={t("gallery.sample.panel_body")} />
            <Toggle checked={true} disabled label={t("gallery.state.disabled")} />
          </div>
        </section>

        <section class="card">
          <h3>{t("gallery.section.fields")}</h3>
          <div class="row">
            <Select options={floors} bind:value={floor} label={t("analyzer.floor")} size="sm" />
            <NumberField bind:value={target} min={-60} max={0} step={0.1} unit="dB" label={t("dialog.normalize.target_label")} />
          </div>
          <div class="grid2">
            <Select options={floors} value={-120} label={t("analyzer.floor")} layout="stacked" disabled />
            <NumberField bind:value={preroll} min={0} max={10} step={0.1} unit="s" label={t("record.preroll")} layout="stacked" width="100%" />
          </div>
          <div class="stack">
            <Slider bind:value={gain} min={-12} max={12} step={0.5} unit="dB" defaultValue={0} label={t("gallery.sample.gain")} />
            <Slider bind:value={threshold} min={-60} max={0} step={1} unit="dB" label={t("gallery.sample.threshold")} />
            <Slider value={30} min={0} max={100} unit="%" label={t("gallery.state.disabled")} disabled />
          </div>
        </section>

        <section class="card flush">
          <h3 class="inset-title">{t("gallery.section.panels")}</h3>
          <div class="panel-demo">
            <PanelHeader title={t("panel.markers.title")} meta={t("gallery.sample.markers_count")}>
              {#snippet actions()}
                <IconButton icon="add" label={t("markers.panel.add")} shortcut={sc.marker} size="sm" />
                <IconButton icon="delete" label={t("markers.panel.delete")} size="sm" />
              {/snippet}
            </PanelHeader>
            <PanelHeader
              title={t("panel.rack.title")}
              meta={t("rack.latency", { ms: "12.0", samples: "576" })}
              collapsible
              bind:expanded={rackOpen}
              controls="gallery-{theme}-rack-body"
            >
              {#snippet actions()}
                <IconButton icon="more" label={t("gallery.sample.more")} size="sm" />
              {/snippet}
            </PanelHeader>
            {#if rackOpen}
              <p id="gallery-{theme}-rack-body" class="panel-body">{t("gallery.sample.panel_body")}</p>
            {/if}
            <Tabs tabs={dockTabs} bind:selected={dock} label={t("gallery.sample.dock")} idPrefix="gallery-{theme}-dock">
              {#snippet panel(id)}
                <p class="panel-body">{dockTabs.find((tab) => tab.id === id)?.label}</p>
              {/snippet}
            </Tabs>
          </div>
        </section>

        <section class="card">
          <h3>{t("gallery.section.status")}</h3>
          <div class="row">
            <Badge>{t("gallery.sample.bypassed")}</Badge>
            <Badge tone="accent">{t("rack.ab")}</Badge>
            <Badge tone="success" icon="check">{t("gallery.sample.pass")}</Badge>
            <Badge tone="warning">{t("gallery.sample.dropouts")}</Badge>
            <Badge tone="danger" icon="error">{t("gallery.sample.fail")}</Badge>
            <Badge tone="record">{t("gallery.state.recording")}</Badge>
          </div>
          <div class="row">
            <Badge variant="solid">{t("gallery.sample.bypassed")}</Badge>
            <Badge variant="solid" tone="accent">{t("rack.ab")}</Badge>
            <Badge variant="solid" tone="success">{t("gallery.sample.pass")}</Badge>
            <Badge variant="solid" tone="warning">{t("gallery.sample.dropouts")}</Badge>
            <Badge variant="solid" tone="danger">{t("gallery.sample.fail")}</Badge>
            <Badge variant="solid" tone="record">{t("gallery.state.recording")}</Badge>
          </div>
          <div class="row">
            <StatusDot tone="success" label={t("gallery.sample.connected")} showLabel />
            <StatusDot tone="warning" label={t("gallery.sample.fallback")} showLabel />
            <StatusDot tone="danger" label={t("gallery.sample.lost")} showLabel />
            <StatusDot tone="record" label={t("gallery.state.recording")} showLabel pulse />
            <StatusDot tone="success" label={t("gallery.sample.connected")} />
          </div>
          <div class="row">
            <Readout label={t("gallery.sample.integrated")} value={-23} unit="LUFS" decimals={1} />
            <Readout label={t("gallery.sample.true_peak")} value={-1.2} unit="dBTP" decimals={1} tone="danger" />
            <Readout label={t("gallery.sample.lra")} value={6.3} unit="LU" decimals={1} />
            <Readout label={t("gallery.sample.gain")} value={3} unit="dB" decimals={1} signed />
          </div>
          <div class="row">
            {#if sc.play}<Kbd keys={sc.play} />{/if}
            {#if sc.record}<Kbd keys={sc.record} />{/if}
            {#if sc.undo}<Kbd keys={sc.undo} />{/if}
            {#if sc.spectral}<Kbd keys={sc.spectral} />{/if}
          </div>
        </section>

        <section class="card flush">
          <h3 class="inset-title">{t("gallery.section.empty")}</h3>
          <div class="empty-demo">
            <EmptyState
              icon="waveform"
              title={t("empty.document.title")}
              description={t("empty.document.body")}
              shortcuts={[
                { label: t("empty.document.shortcut.open"), keys: sc.open ?? "" },
                { label: t("empty.document.shortcut.record"), keys: sc.record ?? "" },
                { label: t("empty.document.shortcut.play"), keys: sc.play ?? "" },
              ]}
              level={3}
            >
              {#snippet actions()}
                <Button variant="primary" icon="open">{t("empty.document.open")}</Button>
                <Button icon="record">{t("empty.document.record")}</Button>
              {/snippet}
            </EmptyState>
          </div>
        </section>

        <section class="card" data-testid="gallery-menus-{theme}">
          <h3>{t("gallery.section.menus")}</h3>
          <div class="menu-demo">
            <Button
              iconEnd="chevronDown"
              aria-haspopup="menu"
              aria-expanded={menuFor === theme}
              bind:element={menuAnchors[theme]}
              onclick={() => (menuFor = menuFor === theme ? null : theme)}
            >
              {t("gallery.sample.menu")}
            </Button>
            <Menu
              open={menuFor === theme}
              anchor={menuAnchors[theme]}
              items={demoMenu}
              label={t("gallery.sample.menu")}
              onclose={() => (menuFor = null)}
            />
            <Button
              aria-haspopup="dialog"
              aria-expanded={popoverFor === theme}
              bind:element={popoverAnchors[theme]}
              onclick={() => (popoverFor = popoverFor === theme ? null : theme)}
            >
              {t("gallery.sample.popover")}
            </Button>
            <Popover
              open={popoverFor === theme}
              anchor={popoverAnchors[theme]}
              label={t("gallery.sample.popover")}
              onclose={() => (popoverFor = null)}
            >
              <p class="hint">{t("gallery.sample.popover_body")}</p>
              <Toggle bind:checked={hearOriginal} label={t("record.hear_original")} />
            </Popover>
          </div>
          {#each FOOTER_PLATFORMS as platform (platform)}
            {@const ordered = orderDialogActions(footerDemo, platform)}
            <div class="footer-demo" data-platform={platform}>
              <span class="footer-label">
                {platform === "windows" ? t("gallery.sample.order_windows") : t("gallery.sample.order_mac_linux")}
              </span>
              <div class="footer-buttons">
                {#each ordered.leading as action (action.label)}
                  <Button size="sm" variant="ghost">{action.label}</Button>
                {/each}
                <span class="footer-spacer"></span>
                {#each ordered.trailing as action (action.label)}
                  <Button size="sm" variant={action.role === "primary" ? "primary" : "secondary"}>{action.label}</Button>
                {/each}
              </div>
            </div>
          {/each}
        </section>

        <section class="card">
          <h3>{t("gallery.section.type")}</h3>
          {#each TYPE_SCALE as size (size)}
            <p class="type-sample" style:font-size="var(--pv-text-{size})" style:line-height="var(--pv-leading-{size})">
              <code>--pv-text-{size}</code>
              {t("gallery.sample.integrated")} {formatWithUnit(-23, "LUFS", 1)}
            </p>
          {/each}
        </section>

        <section class="card">
          <h3>{t("gallery.section.tokens")}</h3>
          <div class="swatches">
            {#each SURFACES as token (token)}
              <div class="swatch surface" style:background="var({token})">
                <span class="aa primary">Aa</span><span class="aa secondary">Aa</span><span class="aa tertiary">Aa</span>
                <code>{token}</code>
              </div>
            {/each}
          </div>
          <div class="swatches small">
            {#each FILLS as token (token)}
              <div class="swatch"><span class="chip" style:background="var({token})"></span><code>{token}</code></div>
            {/each}
          </div>
        </section>

        <section class="card">
          <h3>{t("gallery.section.icons")}</h3>
          <ul class="icon-grid">
            {#each ICON_NAMES as name (name)}
              <li><Icon {name} /><code>{name}</code></li>
            {/each}
          </ul>
        </section>
      </div>
    {/each}
  </div>
</div>

<style>
  .menu-demo {
    display: flex;
    flex-wrap: wrap;
    gap: var(--pv-space-2);
  }

  .footer-demo {
    display: flex;
    flex-direction: column;
    gap: var(--pv-space-1);
    margin-top: var(--pv-space-3);
  }

  .footer-label {
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-xs);
  }

  .footer-buttons {
    display: flex;
    gap: var(--pv-space-2);
    padding: var(--pv-space-2);
    border: var(--pv-border-width) solid var(--pv-border-subtle);
    border-radius: var(--pv-radius-md);
    background: var(--pv-bg-overlay);
  }

  .footer-spacer {
    flex: 1;
  }

  .gallery {
    min-height: 100vh;
    background: var(--pv-bg-app);
    color: var(--pv-text-primary);
    font-family: var(--pv-font-sans);
    font-size: var(--pv-text-md);
    line-height: var(--pv-leading-md);
  }

  .intro {
    padding: var(--pv-space-6) var(--pv-space-6) var(--pv-space-2);
  }

  .intro h1 {
    margin: 0 0 var(--pv-space-1);
    font-size: var(--pv-text-xl);
    line-height: var(--pv-leading-xl);
    font-weight: var(--pv-weight-semibold);
  }

  .intro p {
    margin: 0;
    color: var(--pv-text-secondary);
  }

  .intro .hint {
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-sm);
  }

  .columns {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(560px, 1fr));
  }

  .column {
    display: flex;
    flex-direction: column;
    gap: var(--pv-space-4);
    min-width: 0;
    padding: var(--pv-space-4) var(--pv-space-6) var(--pv-space-8);
    background: var(--pv-bg-app);
    color: var(--pv-text-primary);
  }

  .column-title {
    margin: 0;
    font-size: var(--pv-text-lg);
    line-height: var(--pv-leading-lg);
    font-weight: var(--pv-weight-semibold);
  }

  .card {
    display: flex;
    flex-direction: column;
    gap: var(--pv-space-3);
    padding: var(--pv-space-4);
    border: var(--pv-border-width) solid var(--pv-border);
    border-radius: var(--pv-radius-lg);
    background: var(--pv-bg-panel);
  }

  .card.flush {
    padding: 0;
    overflow: hidden;
  }

  .card h3 {
    margin: 0;
    font-size: var(--pv-text-sm);
    line-height: var(--pv-leading-sm);
    font-weight: var(--pv-weight-semibold);
    color: var(--pv-text-secondary);
  }

  .card .inset-title {
    padding: var(--pv-space-4) var(--pv-space-4) 0;
  }

  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--pv-space-2);
  }

  .stack {
    display: flex;
    flex-direction: column;
    gap: var(--pv-space-3);
  }

  .grid2 {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--pv-space-3);
  }

  /* Transport bar sketch: grouped icon buttons, the time as the chrome's one large element, and
     the on-air tally line when recording. */
  .transport-demo {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--pv-space-2);
    min-height: var(--pv-toolbar-h);
    padding: var(--pv-space-1) var(--pv-space-2);
    border: var(--pv-border-width) solid var(--pv-border);
    border-radius: var(--pv-radius-md);
    background: var(--pv-bg-panel);
    transition: box-shadow var(--pv-duration-base) var(--pv-ease-standard);
  }

  .transport-demo.on-air {
    box-shadow: inset 0 2px 0 var(--pv-record);
  }

  .group {
    display: flex;
    align-items: center;
    gap: var(--pv-space-1);
  }

  .spacer {
    flex: 1;
  }

  .panel-demo {
    display: flex;
    flex-direction: column;
    margin-top: var(--pv-space-3);
    border-top: var(--pv-border-width) solid var(--pv-border);
  }

  .panel-body {
    margin: 0;
    padding: var(--pv-space-3);
    color: var(--pv-text-secondary);
    font-size: var(--pv-text-sm);
  }

  .empty-demo {
    display: flex;
    min-height: 320px;
    margin-top: var(--pv-space-3);
    background: var(--pv-bg-inset);
  }

  .type-sample {
    display: flex;
    align-items: baseline;
    gap: var(--pv-space-3);
    margin: 0;
    font-variant-numeric: tabular-nums;
  }

  .type-sample code,
  .swatch code,
  .icon-grid code {
    font-family: var(--pv-font-mono);
    font-size: var(--pv-text-xs);
    color: var(--pv-text-tertiary);
  }

  .type-sample code {
    min-width: 8.5rem;
  }

  .swatches {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: var(--pv-space-2);
  }

  .swatches.small {
    grid-template-columns: repeat(auto-fill, minmax(190px, 1fr));
  }

  .swatch {
    display: flex;
    align-items: center;
    gap: var(--pv-space-2);
    padding: var(--pv-space-2);
    border: var(--pv-border-width) solid var(--pv-border);
    border-radius: var(--pv-radius-md);
  }

  .swatch.surface {
    flex-wrap: wrap;
  }

  .aa {
    font-weight: var(--pv-weight-semibold);
  }

  .aa.primary {
    color: var(--pv-text-primary);
  }

  .aa.secondary {
    color: var(--pv-text-secondary);
  }

  .aa.tertiary {
    color: var(--pv-text-tertiary);
  }

  .swatch.surface code {
    flex-basis: 100%;
  }

  .chip {
    flex: none;
    width: 20px;
    height: 20px;
    border: var(--pv-border-width) solid var(--pv-border);
    border-radius: var(--pv-radius-sm);
  }

  .icon-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(120px, 1fr));
    gap: var(--pv-space-1);
    margin: 0;
    padding: 0;
    list-style: none;
    color: var(--pv-text-secondary);
  }

  .icon-grid li {
    display: flex;
    align-items: center;
    gap: var(--pv-space-2);
    padding: var(--pv-space-1);
  }
</style>
