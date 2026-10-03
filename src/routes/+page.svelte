<script lang="ts">
  import { isTauri } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { open, save } from "@tauri-apps/plugin-dialog";
  import { onMount } from "svelte";
  import Confirm from "$lib/Confirm.svelte";
  import HexView from "$lib/HexView.svelte";
  import {
    blankCheck,
    deviceInfo,
    eraseChip,
    errorText,
    loadFile,
    programmerStatus,
    readId,
    readMemory,
    saveBuffer,
    searchDevices,
    verifyFile,
    writeFile,
    type DeviceInfo,
    type FileReport,
    type LogEvent,
    type ProgressEvent,
    type SearchHit,
  } from "$lib/api";

  const profiles = ["TL866II+", "TL866A"];

  let profile = $state("TL866II+");
  let connected = $state(false);
  let connectedModel = $state<string | null>(null);
  let firmware = $state<string | null>(null);
  let serialNumber = $state<string | null>(null);
  let statusMessage = $state("Looking for a programmer...");
  let warning = $state<string | null>(null);
  let query = $state("");
  let debouncedQuery = $state("");
  let hits = $state<SearchHit[]>([]);
  let truncated = $state(false);
  let searching = $state(false);
  let selected = $state<string | null>(null);
  let info = $state<DeviceInfo | null>(null);
  let page = $state(0);
  let filePath = $state<string | null>(null);
  let fileReport = $state<FileReport | null>(null);
  let bufferLength = $state(0);
  let bufferGeneration = $state(0);
  let working = $state(false);
  let done = $state(0);
  let total = $state(0);
  let lines = $state<string[]>([]);
  let logEl = $state<HTMLPreElement | null>(null);
  let confirm = $state<null | { title: string; body: string; label: string; run: () => Promise<void> }>(
    null,
  );

  let activeModel = $derived(connectedModel ?? profile);
  let memoryBytes = $derived(
    info ? (page === 1 ? info.dataBytes : info.codeBytes) : 0,
  );
  let fileReady = $derived(
    Boolean(
      info &&
        filePath &&
        fileReport?.matches &&
        fileReport.memoryBytes === memoryBytes &&
        memoryBytes > 0,
    ),
  );
  let percent = $derived(total > 0 ? Math.min(100, Math.round((done / total) * 100)) : 0);

  $effect(() => {
    const next = query;
    const timer = setTimeout(() => {
      debouncedQuery = next;
    }, 200);
    return () => clearTimeout(timer);
  });

  $effect(() => {
    const q = debouncedQuery;
    const model = activeModel;
    if (q.trim().length < 2) {
      hits = [];
      truncated = false;
      searching = false;
      return;
    }
    let cancelled = false;
    searching = true;
    searchDevices(q, model)
      .then((response) => {
        if (cancelled) return;
        hits = response.hits;
        truncated = response.truncated;
      })
      .catch((err) => {
        if (!cancelled) note(errorText(err));
      })
      .finally(() => {
        if (!cancelled) searching = false;
      });
    return () => {
      cancelled = true;
    };
  });

  $effect(() => {
    const name = selected;
    const model = activeModel;
    if (!name) {
      info = null;
      return;
    }
    let cancelled = false;
    deviceInfo(name, model)
      .then((next) => {
        if (!cancelled) info = next;
      })
      .catch((err) => {
        if (cancelled) return;
        info = null;
        selected = null;
        note(errorText(err));
      });
    return () => {
      cancelled = true;
    };
  });

  $effect(() => {
    if (info && info.dataBytes === 0 && page === 1) page = 0;
  });

  $effect(() => {
    const path = filePath;
    const name = selected ?? "";
    const model = activeModel;
    const memoryPage = page;
    if (!path) {
      fileReport = null;
      return;
    }
    let cancelled = false;
    loadFile(path, name, memoryPage, model)
      .then((report) => {
        if (cancelled) return;
        fileReport = report;
        bufferLength = report.bufferLength;
        bufferGeneration += 1;
        note(report.message);
      })
      .catch((err) => {
        if (cancelled) return;
        fileReport = null;
        note(errorText(err));
      });
    return () => {
      cancelled = true;
    };
  });

  $effect(() => {
    void lines.length;
    if (logEl) logEl.scrollTop = logEl.scrollHeight;
  });

  onMount(() => {
    if (!isTauri()) return;
    const unsubs: Array<() => void> = [];
    let cancelled = false;
    void (async () => {
      const progressUnsub = await listen<ProgressEvent>("progress", (event) => {
        done = event.payload.done;
        total = event.payload.total;
      });
      const logUnsub = await listen<LogEvent>("log", (event) => {
        note(event.payload.message);
      });
      if (cancelled) {
        progressUnsub();
        logUnsub();
        return;
      }
      unsubs.push(progressUnsub, logUnsub);
      await refresh(true);
    })();
    return () => {
      cancelled = true;
      for (const unsub of unsubs) unsub();
    };
  });

  function note(message: string) {
    if (!message || lines.at(-1) === message) return;
    lines = [...lines, message].slice(-400);
  }

  async function refresh(force = false) {
    try {
      const status = await programmerStatus(force);
      connected = status.connected;
      connectedModel = status.connected ? status.model : null;
      firmware = status.firmware;
      serialNumber = status.serialNumber;
      statusMessage = status.message;
      warning = status.warning;
    } catch (err) {
      connected = false;
      connectedModel = null;
      statusMessage = errorText(err);
      note(statusMessage);
    }
  }

  async function chooseFile() {
    const selectedPath = await open({
      multiple: false,
      directory: false,
      filters: [
        {
          name: "Firmware",
          extensions: ["bin", "hex", "ihx", "srec", "s19", "s28", "s37"],
        },
      ],
    });
    if (typeof selectedPath === "string") filePath = selectedPath;
  }

  async function saveRead() {
    const path = await save({
      filters: [
        { name: "Binary", extensions: ["bin"] },
        { name: "Intel HEX", extensions: ["hex"] },
        { name: "S-record", extensions: ["srec"] },
      ],
    });
    if (typeof path !== "string") return;
    try {
      await saveBuffer(path);
    } catch (err) {
      note(errorText(err));
    }
  }

  async function run(action: () => Promise<void>) {
    if (working) return;
    working = true;
    done = 0;
    total = 0;
    try {
      await action();
    } catch (err) {
      note(errorText(err));
    } finally {
      working = false;
      await refresh(false);
    }
  }

  function doRead() {
    if (!selected) return;
    const name = selected;
    const memoryPage = page;
    run(async () => {
      const report = await readMemory(name, memoryPage);
      bufferLength = report.bufferLength;
      bufferGeneration += 1;
    });
  }

  function doVerify() {
    if (!selected || !filePath) return;
    const name = selected;
    const memoryPage = page;
    const path = filePath;
    run(async () => {
      await verifyFile(name, memoryPage, path);
    });
  }

  function doBlank() {
    if (!selected) return;
    const name = selected;
    run(async () => {
      await blankCheck(name);
    });
  }

  function doReadId() {
    if (!selected) return;
    const name = selected;
    run(async () => {
      await readId(name);
    });
  }

  function askWrite() {
    if (!selected || !filePath || !info) return;
    const name = selected;
    const path = filePath;
    const bytes = memoryBytes;
    const memoryPage = page;
    const label = memoryPage === 1 ? "data memory" : "code memory";
    confirm = {
      title: `Write ${name}?`,
      body: `${path}\n\n${bytes} bytes will be programmed to ${label}. The chip ID is checked first.`,
      label: "Write",
      run: async () => {
        await writeFile(name, memoryPage, path, true);
      },
    };
  }

  function askErase() {
    if (!selected || !info) return;
    const name = selected;
    const bytes = info.codeBytes;
    confirm = {
      title: `Erase ${name}?`,
      body: `This electrically erases the chip. Code memory is ${bytes} bytes.`,
      label: "Erase",
      run: async () => {
        await eraseChip(name, true);
      },
    };
  }

  async function confirmAction() {
    const pending = confirm;
    confirm = null;
    if (!pending) return;
    await run(pending.run);
  }
</script>

<div class="app">
  <header class="top">
    <h1>miniproui</h1>
    <div class="status">
      {#if connected}
        <span class="ok">Connected</span>
        <strong>{connectedModel}</strong>
        {#if firmware}<span class="muted">firmware {firmware}</span>{/if}
        {#if serialNumber}<span class="muted">serial {serialNumber}</span>{/if}
      {:else}
        {statusMessage}
      {/if}
    </div>
    <button class="btn" disabled={working} onclick={() => refresh(true)}>Rescan</button>
  </header>

  {#if warning}
    <p class="warning">{warning}</p>
  {/if}

  <div class="body">
    <aside class="devices">
      {#if !connected}
        <label>
          <span class="muted">Database</span>
          <select class="select" bind:value={profile}>
            {#each profiles as item (item)}
              <option value={item}>{item === "TL866II+" ? "TL866II+, T48, T56, T76" : "TL866A / CS"}</option>
            {/each}
          </select>
        </label>
      {/if}

      <div class="search-row">
        <input class="text" placeholder="Search devices, e.g. ATMEGA48" bind:value={query} />
      </div>

      <div class="results">
        {#if query.trim().length < 2}
          <p class="muted">Type at least 2 characters.</p>
        {:else if searching && hits.length === 0}
          <p class="muted">Searching...</p>
        {:else if hits.length === 0}
          <p class="muted">No matches.</p>
        {:else}
          {#each hits as hit (hit.name)}
            <button
              class="hit"
              class:selected={selected === hit.name}
              onclick={() => (selected = hit.name)}
            >
              <span class="name">{hit.name}</span>
              {#if hit.manufacturer}<span class="mfr">{hit.manufacturer}</span>{/if}
            </button>
          {/each}
          {#if truncated}
            <p class="muted">Showing 200 matches. Refine the search.</p>
          {/if}
        {/if}
      </div>

      {#if info}
        <div class="info">
          <strong>{info.name}</strong><br />
          {#if info.manufacturer}{info.manufacturer} · {/if}{info.package}<br />
          code {info.codeBytes} bytes
          {#if info.dataBytes > 0} · data {info.dataBytes} bytes{/if}<br />
          ID {info.chipId} · blank {info.blankValue}
        </div>
      {/if}
    </aside>

    <section class="main">
      <div class="ops">
        <button class="btn primary" disabled={!connected || !selected || working} onclick={doRead}>Read</button>
        <button class="btn danger" disabled={!connected || !fileReady || working} onclick={askWrite}>Write</button>
        <button class="btn" disabled={!connected || !fileReady || working} onclick={doVerify}>Verify</button>
        <button class="btn danger" disabled={!connected || !info?.canErase || working} onclick={askErase}>Erase</button>
        <button class="btn" disabled={!connected || !selected || working} onclick={doBlank}>Blank check</button>
        <button class="btn" disabled={!connected || !selected || working} onclick={doReadId}>Read ID</button>
      </div>

      <div class="file-row">
        <span class="path">{filePath ?? "No file selected"}</span>
        <button class="btn" disabled={working} onclick={chooseFile}>Open</button>
        <button class="btn" disabled={working || bufferLength === 0} onclick={saveRead}>Save</button>
      </div>

      {#if info && info.dataBytes > 0}
        <div class="page-row">
          <span class="muted">Memory</span>
          <select
            class="select"
            value={page}
            disabled={working}
            onchange={(event) => {
              const target = event.currentTarget;
              if (target instanceof HTMLSelectElement) page = Number(target.value);
            }}
          >
            <option value={0}>Code ({info.codeBytes} bytes)</option>
            <option value={1}>Data ({info.dataBytes} bytes)</option>
          </select>
        </div>
      {/if}

      {#if fileReport && !fileReport.matches}
        <p class="banner">{fileReport.message}</p>
      {/if}

      {#if working}
        <div class="progress" aria-valuemin="0" aria-valuemax="100" aria-valuenow={percent}>
          <div class="bar" class:indeterminate={total === 0} style:width={total === 0 ? undefined : `${percent}%`}></div>
        </div>
        <div class="muted">
          {#if total > 0}{done} / {total} bytes{:else}Working...{/if}
        </div>
      {/if}

      <div class="hex-wrap">
        {#key bufferGeneration}
          <HexView length={bufferLength} />
        {/key}
      </div>

      <pre class="log" bind:this={logEl}>{lines.join("\n")}</pre>
    </section>
  </div>
</div>

{#if confirm}
  <Confirm
    title={confirm.title}
    body={confirm.body}
    confirmLabel={confirm.label}
    onConfirm={confirmAction}
    onCancel={() => (confirm = null)}
  />
{/if}
