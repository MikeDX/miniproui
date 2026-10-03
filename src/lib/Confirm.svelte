<script lang="ts">
  interface Props {
    title: string;
    body: string;
    confirmLabel: string;
    onConfirm: () => void;
    onCancel: () => void;
  }

  let { title, body, confirmLabel, onConfirm, onCancel }: Props = $props();
  let cancel = $state<HTMLButtonElement | null>(null);

  function onKeydown(event: KeyboardEvent) {
    if (event.key === "Escape") onCancel();
  }

  $effect(() => {
    cancel?.focus();
  });
</script>

<svelte:window onkeydown={onKeydown} />

<div class="backdrop" role="presentation">
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="confirm-title">
    <h2 id="confirm-title">{title}</h2>
    <p>{body}</p>
    <div class="actions">
      <button class="btn" bind:this={cancel} onclick={onCancel}>Cancel</button>
      <button class="btn danger" onclick={onConfirm}>{confirmLabel}</button>
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.55);
    display: grid;
    place-items: center;
    z-index: 20;
  }

  .dialog {
    width: min(440px, calc(100vw - 32px));
    background: #2a2e3a;
    color: #e7e9ee;
    border: 1px solid #4a5163;
    border-radius: 10px;
    padding: 18px 18px 14px;
    box-shadow: 0 16px 50px rgba(0, 0, 0, 0.4);
  }

  h2 {
    margin: 0 0 8px;
    font-size: 18px;
  }

  p {
    margin: 0;
    white-space: pre-wrap;
    color: #c5cad6;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 16px;
  }
</style>
