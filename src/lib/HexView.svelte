<script lang="ts">
  import { hexWindow } from "$lib/api";

  let { length }: { length: number } = $props();

  const rowBytes = 16;
  const rowHeight = 18;
  let scrollTop = $state(0);
  let viewport = $state(240);
  let scroller = $state<HTMLDivElement | null>(null);
  let windowOffset = $state(0);
  let windowBytes = $state<number[]>([]);

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
                {hexCell(bytes[0])} {hexCell(bytes[1])} {hexCell(bytes[2])} {hexCell(bytes[3])}
                {hexCell(bytes[4])} {hexCell(bytes[5])} {hexCell(bytes[6])} {hexCell(bytes[7])}
                <span class="gap"></span>
                {hexCell(bytes[8])} {hexCell(bytes[9])} {hexCell(bytes[10])} {hexCell(bytes[11])}
                {hexCell(bytes[12])} {hexCell(bytes[13])} {hexCell(bytes[14])} {hexCell(bytes[15])}
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
</style>
