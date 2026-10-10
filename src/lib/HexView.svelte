<script lang="ts">
  import { hexWindow } from "$lib/api";

  let {
    length,
    editable = false,
    onPatch,
  }: {
    length: number;
    editable?: boolean;
    onPatch?: (offset: number, value: number) => Promise<void>;
  } = $props();

  const rowBytes = 16;
  const rowHeight = 18;
  let scrollTop = $state(0);
  let viewport = $state(240);
  let scroller = $state<HTMLDivElement | null>(null);
  let windowOffset = $state(0);
  let windowBytes = $state<number[]>([]);
  let editOffset = $state<number | null>(null);
  let nibble = $state("");

  let rows = $derived(Math.ceil(length / rowBytes));
  let start = $derived(Math.max(0, Math.floor(scrollTop / rowHeight) - 8));
  let visible = $derived(Math.ceil(viewport / rowHeight) + 16);
  let rowIndexes = $derived(
    Array.from({ length: Math.min(visible, Math.max(0, rows - start)) }, (_, index) => start + index),
  );

  $effect(() => {
    const offset = start * rowBytes;
    const count = visible * rowBytes;
    const total = length;
    if (total === 0) {
      windowOffset = 0;
      windowBytes = [];
      return;
    }
    let cancelled = false;
    hexWindow(offset, count).then((window) => {
      if (!cancelled) {
        windowOffset = window.offset;
        windowBytes = window.bytes;
      }
    });
    return () => {
      cancelled = true;
    };
  });

  function onScroll(event: Event) {
    const target = event.currentTarget;
    if (target instanceof HTMLDivElement) {
      scrollTop = target.scrollTop;
      viewport = target.clientHeight;
    }
  }

  function rowBytesAt(row: number): number[] {
    const offset = row * rowBytes;
    const startIndex = offset - windowOffset;
    if (startIndex < 0 || startIndex >= windowBytes.length) return [];
    return windowBytes.slice(startIndex, startIndex + rowBytes);
  }

  function hexCell(value: number | undefined): string {
    return value === undefined ? "  " : value.toString(16).padStart(2, "0");
  }

  function asciiCell(value: number | undefined): string {
    if (value === undefined) return " ";
    return value >= 0x20 && value <= 0x7e ? String.fromCharCode(value) : ".";
  }

  function cellText(offset: number, value: number | undefined): string {
    if (editOffset === offset && nibble.length > 0) return `${nibble}_`.slice(0, 2);
    return hexCell(value);
  }

  function beginEdit(offset: number) {
    if (!editable || offset >= length) return;
    editOffset = offset;
    nibble = "";
  }

  async function onKey(event: KeyboardEvent) {
    if (!editable || editOffset === null) return;
    const key = event.key.toLowerCase();
    if (key === "escape") {
      editOffset = null;
      nibble = "";
      return;
    }
    if (!/^[0-9a-f]$/.test(key)) return;
    event.preventDefault();
    nibble += key;
    if (nibble.length < 2) return;
    const value = Number.parseInt(nibble, 16);
    const offset = editOffset;
    nibble = "";
    editOffset = null;
    const index = offset - windowOffset;
    const previous = windowBytes[index];
    if (index >= 0 && index < windowBytes.length) windowBytes[index] = value;
    try {
      await onPatch?.(offset, value);
    } catch {
      if (index >= 0 && previous !== undefined) windowBytes[index] = previous;
    }
  }
</script>

<div class="hex">
  <div class="hex-row head">
    <span class="offset">Offset</span>
    <span class="bytes">00 01 02 03 04 05 06 07<span class="gap"></span>08 09 0A 0B 0C 0D 0E 0F</span>
    <span class="ascii">ASCII</span>
  </div>
  {#if length === 0}
    <p class="empty">Read a chip or open a file to view bytes.</p>
  {:else}
    <div
      class="hex-scroll"
      bind:this={scroller}
      bind:clientHeight={viewport}
      onscroll={onScroll}
    >
      <div class="hex-spacer" style:height="{rows * rowHeight}px">
        <div class="hex-rows" style:transform="translateY({start * rowHeight}px)">
          {#each rowIndexes as row (row)}
            {@const bytes = rowBytesAt(row)}
            <div class="hex-row" style:height="{rowHeight}px">
              <span class="offset">{(row * rowBytes).toString(16).padStart(8, "0")}</span>
              <span class="bytes">
                {#each [0, 1, 2, 3, 4, 5, 6, 7] as column}
                  {@const offset = row * rowBytes + column}
                  <button
                    type="button"
                    class="cell"
                    class:editing={editOffset === offset}
                    onclick={() => beginEdit(offset)}
                    onkeydown={onKey}
                  >{cellText(offset, bytes[column])}</button>
                {/each}
                <span class="gap"></span>
                {#each [8, 9, 10, 11, 12, 13, 14, 15] as column}
                  {@const offset = row * rowBytes + column}
                  <button
                    type="button"
                    class="cell"
                    class:editing={editOffset === offset}
                    onclick={() => beginEdit(offset)}
                    onkeydown={onKey}
                  >{cellText(offset, bytes[column])}</button>
                {/each}
              </span>
              <span class="ascii">{bytes.map(asciiCell).join("")}</span>
            </div>
          {/each}
        </div>
      </div>
    </div>
  {/if}
</div>

<style>
  .hex {
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: #12141a;
    border: 1px solid #343846;
    border-radius: 8px;
    overflow: hidden;
  }

  .hex-row {
    font-family: ui-monospace, "SF Mono", Menlo, Consolas, monospace;
    font-size: 12px;
    line-height: 18px;
    white-space: pre;
  }

  .hex-row.head {
    color: #8b93a7;
    background: #1a1d27;
    border-bottom: 1px solid #343846;
  }

  .empty {
    margin: 0;
    padding: 24px 12px;
    color: #8b93a7;
  }

  .hex-scroll {
    flex: 1;
    min-height: 180px;
    overflow: auto;
  }

  .hex-spacer {
    position: relative;
  }

  .hex-rows {
    position: absolute;
    left: 0;
    right: 0;
    top: 0;
  }

  .hex-row {
    display: grid;
    grid-template-columns: 7.2em 1fr auto;
    gap: 12px;
    padding: 0 10px;
    color: #d5d8e2;
  }

  .bytes {
    white-space: normal;
  }

  .offset {
    color: #7f8cff;
  }

  .gap {
    display: inline-block;
    width: 0.6em;
  }

  .ascii {
    color: #b7c3a1;
  }

  .cell {
    font: inherit;
    color: inherit;
    background: transparent;
    border: 0;
    padding: 0;
    min-width: 1.35em;
    margin-right: 0.2em;
  }

  .cell.editing {
    background: #31568f;
    border-radius: 2px;
  }
</style>
