<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";

  let canvas = $state<HTMLCanvasElement | null>(null);

  let windowHandles = $state<string[]>([]);
  let selectedWindow = $state<string>("");

  async function refreshHandles() {
    // Replace with your Tauri command to get window handles
    windowHandles = await invoke<string[]>("list_window_handles");
    if (windowHandles.length > 0) {
      selectedWindow = windowHandles[0];
    }
    await invoke("set_window", { title: selectedWindow });
  }

  async function updateSelectedWindow() {
    await invoke("set_window", { title: selectedWindow });
  }

  onMount(() => {
    refreshHandles();
    listen<{ buffer: Uint8Array; width: number; height: number }>(
      "minimap-update",
      async (event) => {
        const { buffer, width, height } = event.payload;
        if (canvas) {
          const ctx = canvas.getContext("2d");
          if (ctx) {
            const imageData = new ImageData(
              new Uint8ClampedArray(buffer),
              width,
              height,
            );
            const bitmap = await createImageBitmap(imageData);
            ctx.drawImage(
              bitmap,
              0,
              0,
              width,
              height,
              0,
              0,
              canvas.width,
              canvas.height,
            );
          }
        }
      },
    );
  });
</script>

<main class="container">
  <h1>Starry Bot</h1>

  <div class="grid-layout">
    <div class="column">
      <h2>Minimap</h2>
      <canvas bind:this={canvas} width="400" height="300"></canvas>
    </div>
    <div class="column">
      <h2>Other Content</h2>
      <label for="window-select">Select Window:</label>
      <select id="window-select" bind:value={selectedWindow} onchange={updateSelectedWindow}>
        {#each windowHandles as handle}
          <option value={handle}>{handle}</option>
        {/each}
      </select>
      <button onclick={refreshHandles}>Refresh Handles</button>
    </div>
  </div>
</main>

<style>
  :root {
    font-family: Inter, Avenir, Helvetica, Arial, sans-serif;
    font-size: 16px;
    line-height: 24px;
    font-weight: 400;

    color: #0f0f0f;
    background-color: #f6f6f6;

    font-synthesis: none;
    text-rendering: optimizeLegibility;
    -webkit-font-smoothing: antialiased;
    -moz-osx-font-smoothing: grayscale;
    -webkit-text-size-adjust: 100%;
  }

  select#window-select {
    width: 200px;
    padding: 0.5em;
    font-size: 1em;
  }

  .container {
    margin: 0;
    padding-top: 1vh;
    display: flex;
    flex-direction: column;
    justify-content: center;
    text-align: center;
  }

  .grid-layout {
    display: grid;
    grid-template-columns: 420px 1fr;
    gap: 2rem;
    justify-content: center;
    align-items: flex-start;
    margin-top: 2rem;
  }

  .column {
    display: flex;
    flex-direction: column;
    justify-content: center;
    align-items: center;
    width: 500px;
  }

  .row {
    display: flex;
    justify-content: center;
  }

  a {
    font-weight: 500;
    color: #646cff;
    text-decoration: inherit;
  }

  a:hover {
    color: #535bf2;
  }

  h1 {
    text-align: center;
  }

  input,
  button {
    border-radius: 8px;
    border: 1px solid transparent;
    padding: 0.6em 1.2em;
    font-size: 1em;
    font-weight: 500;
    font-family: inherit;
    color: #0f0f0f;
    background-color: #ffffff;
    transition: border-color 0.25s;
    box-shadow: 0 2px 2px rgba(0, 0, 0, 0.2);
  }

  button {
    cursor: pointer;
  }

  button:hover {
    border-color: #396cd8;
  }
  button:active {
    border-color: #396cd8;
    background-color: #e8e8e8;
  }

  input,
  button {
    outline: none;
  }

  @media (prefers-color-scheme: dark) {
    :root {
      color: #f6f6f6;
      background-color: #2f2f2f;
    }

    a:hover {
      color: #24c8db;
    }

    input,
    button {
      color: #ffffff;
      background-color: #0f0f0f98;
    }
    button:active {
      background-color: #0f0f0f69;
    }
  }
</style>
