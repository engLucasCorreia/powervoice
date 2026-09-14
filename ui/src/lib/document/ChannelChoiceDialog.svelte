<script lang="ts">
  import type { DownmixChoiceDto } from "../ipc/bindings";
  import { t } from "../i18n";
  import { Button, Dialog, Icon } from "../ui";
  import { documentState, resolveChannelChoicePrompt } from "./document.svelte";

  /**
   * T-209 (SPEC-005 §2.4): "Open stereo file" — asks whether multichannel input becomes mono by
   * averaging every channel or by picking one, with the silent-channel hint preselecting the
   * active side when the probe found one channel silent and the other active. "Always do this
   * for multichannel files" remembers the choice as `multichannel_policy` (Settings → Files).
   * H-25: Dialog shell.
   */
  const doc = documentState();

  let mode = $state<"average" | "channel">("average");
  let channelIndex = $state(0);
  let remember = $state(false);

  $effect(() => {
    const probe = doc.channelChoicePrompt?.probe;
    if (!probe) {
      return;
    }
    if (probe.suggested_channel !== null) {
      mode = "channel";
      channelIndex = probe.suggested_channel;
    } else {
      mode = "average";
      channelIndex = 0;
    }
    remember = false;
  });

  function channelLabel(index: number): string {
    return doc.channelChoicePrompt?.probe.channels[index]?.label ?? `Channel ${index + 1}`;
  }

  // SPEC-005 §2.4's silent-channel hint text names both sides; the probe only ever suggests a
  // channel for exactly this two-channel "one side is silent" case in practice, so the hint is
  // shown for that shape only (a > 2-channel suggestion — e.g. 5.1 — shows no hint text, matching
  // the spec's stereo-focused example).
  function silentHint(): string | null {
    const probe = doc.channelChoicePrompt?.probe;
    if (!probe || probe.suggested_channel === null || probe.channels.length !== 2) {
      return null;
    }
    const active = probe.suggested_channel;
    const silent = active === 0 ? 1 : 0;
    return t("dialog.channel_choice.silent_hint", {
      channel: channelLabel(silent),
      other: channelLabel(active),
    });
  }

  function confirm(): void {
    const choice: DownmixChoiceDto =
      mode === "average" ? { kind: "average" } : { kind: "channel", index: channelIndex };
    resolveChannelChoicePrompt({ choice, remember });
  }

  function cancel(): void {
    resolveChannelChoicePrompt(null);
  }

  function onKeydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === "Escape") {
      cancel();
    }
  }
</script>

{#if doc.channelChoicePrompt}
  {@const probe = doc.channelChoicePrompt.probe}
  {@const hint = silentHint()}
  <Dialog
    title={probe.channels.length === 2
      ? t("dialog.channel_choice.title_stereo")
      : t("dialog.channel_choice.title_multi", { count: probe.channels.length })}
    titleId="channel-choice-title"
    testid="channel-choice-dialog"
    onkeydown={onKeydown}
  >
    <p>{t("dialog.channel_choice.message", { count: probe.channels.length })}</p>
    {#if hint}
      <p class="suggestion" data-testid="channel-choice-silent-hint">
        <Icon name="info" size="sm" />
        <span>{hint}</span>
      </p>
    {/if}
    <fieldset class="choices">
      <label class="option">
        <input
          type="radio"
          name="channel-choice-mode"
          value="average"
          checked={mode === "average"}
          onchange={() => (mode = "average")}
        />
        {t("dialog.channel_choice.mix")}
      </label>
      <label class="option">
        <input
          type="radio"
          name="channel-choice-mode"
          value="channel"
          checked={mode === "channel"}
          onchange={() => (mode = "channel")}
        />
        {t("dialog.channel_choice.use_channel")}
        <select
          data-testid="channel-choice-select"
          disabled={mode !== "channel"}
          value={channelIndex}
          onchange={(e) => {
            mode = "channel";
            channelIndex = Number(e.currentTarget.value);
          }}
        >
          {#each probe.channels as channel, index (index)}
            <option value={index}>{channel.label}</option>
          {/each}
        </select>
      </label>
    </fieldset>
    <label class="option remember">
      <input
        type="checkbox"
        data-testid="channel-choice-remember"
        checked={remember}
        onchange={(e) => (remember = e.currentTarget.checked)}
      />
      {t("dialog.channel_choice.remember")}
    </label>
    {#snippet footer()}
      <Button testid="channel-choice-cancel" onclick={cancel}>
        {t("dialog.channel_choice.cancel")}
      </Button>
      <Button variant="primary" testid="channel-choice-open" onclick={confirm}>
        {t("dialog.channel_choice.open")}
      </Button>
    {/snippet}
  </Dialog>
{/if}

<style>
  .suggestion {
    display: flex;
    align-items: flex-start;
    gap: var(--pv-space-2);
    padding: var(--pv-space-2) var(--pv-space-3);
    border-radius: var(--pv-radius-md);
    background: var(--pv-accent-soft);
    color: var(--pv-accent-text);
    font-size: var(--pv-text-sm);
    line-height: var(--pv-leading-sm);
  }

  .suggestion :global(svg) {
    flex: none;
    margin-top: 1px;
  }

  .choices {
    gap: var(--pv-space-1);
  }

  .remember {
    color: var(--pv-text-secondary);
  }
</style>
