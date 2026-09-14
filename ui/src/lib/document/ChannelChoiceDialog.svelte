<script lang="ts">
  import type { DownmixChoiceDto } from "../ipc/bindings";
  import { t } from "../i18n";
  import { documentState, resolveChannelChoicePrompt } from "./document.svelte";

  /**
   * T-209 (SPEC-005 §2.4): "Open stereo file" — asks whether multichannel input becomes mono by
   * averaging every channel or by picking one, with the silent-channel hint preselecting the
   * active side when the probe found one channel silent and the other active. "Always do this
   * for multichannel files" remembers the choice as `multichannel_policy` (Settings → Files).
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
  <div class="backdrop">
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <div
      class="dialog"
      role="dialog"
      aria-modal="true"
      aria-labelledby="channel-choice-title"
      data-testid="channel-choice-dialog"
      tabindex="-1"
      onkeydown={onKeydown}
    >
      <h2 id="channel-choice-title">
        {probe.channels.length === 2
          ? t("dialog.channel_choice.title_stereo")
          : t("dialog.channel_choice.title_multi", { count: probe.channels.length })}
      </h2>
      <p>{t("dialog.channel_choice.message", { count: probe.channels.length })}</p>
      {#if hint}
        <p class="hint" data-testid="channel-choice-silent-hint">{hint}</p>
      {/if}
      <fieldset>
        <label>
          <input
            type="radio"
            name="channel-choice-mode"
            value="average"
            checked={mode === "average"}
            onchange={() => (mode = "average")}
          />
          {t("dialog.channel_choice.mix")}
        </label>
        <label>
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
      <label class="remember">
        <input
          type="checkbox"
          data-testid="channel-choice-remember"
          checked={remember}
          onchange={(e) => (remember = e.currentTarget.checked)}
        />
        {t("dialog.channel_choice.remember")}
      </label>
      <div class="actions">
        <button type="button" data-testid="channel-choice-cancel" onclick={cancel}>
          {t("dialog.channel_choice.cancel")}
        </button>
        <button type="button" class="primary" data-testid="channel-choice-open" onclick={confirm}>
          {t("dialog.channel_choice.open")}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(0, 0, 0, 0.45);
    z-index: 1000;
  }

  .dialog {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    min-width: 26rem;
    max-width: 90vw;
    padding: 1rem 1.25rem;
    background: var(--surface-panel);
    border: 1px solid var(--surface-border);
    border-radius: 6px;
    color: var(--text-primary);
  }

  h2 {
    margin: 0;
    font-size: 1rem;
  }

  p {
    margin: 0;
    color: var(--text-secondary);
  }

  p.hint {
    color: var(--accent, var(--text-secondary));
  }

  fieldset {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    border: 1px solid var(--surface-border);
    border-radius: 4px;
    padding: 0.5rem 0.75rem;
  }

  label {
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }

  label.remember {
    color: var(--text-secondary);
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
  }

  button {
    background: var(--surface-panel-raised);
    color: var(--text-primary);
    border: 1px solid var(--surface-border);
    border-radius: 4px;
    padding: 0.25rem 0.75rem;
  }

  button.primary {
    border-color: var(--accent);
    color: var(--accent);
  }
</style>
