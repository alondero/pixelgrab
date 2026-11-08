// Opt-in Windows acceptance against the packaged production binary, real IPC,
// real capture, Windows clipboard/dialogs and OLE. Never run this in CI.
import assert from "node:assert/strict";
import { spawn, execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdir, mkdtemp, readFile, access } from "node:fs/promises";
import { resolve, join, dirname } from "node:path";
import { chromium } from "playwright";

const exec = promisify(execFile);
const binary = process.argv[2];
if (
  process.env.CI ||
  process.platform !== "win32" ||
  !binary ||
  !process.argv.includes("--real-desktop")
) {
  throw new Error("Usage: node tests/native/acceptance.mjs <pixelgrab.exe> --real-desktop");
}
const fixture = resolve("tests/native/windows-fixture.ps1");
await mkdir("fixtures/local", { recursive: true });
const root = await mkdtemp(resolve("fixtures/local/native-"));
const port = 9248;
const children = [];
const delay = (ms) => new Promise((done) => setTimeout(done, ms));
async function until(check, message) {
  for (let i = 0; i < 150; i++) {
    if (await check().catch(() => false)) return;
    await delay(100);
  }
  throw new Error(message);
}
function helperArgs(mode, args) {
  return [
    "-NoProfile",
    "-STA",
    "-ExecutionPolicy",
    "Bypass",
    "-File",
    fixture,
    "-Mode",
    mode,
    ...args,
  ];
}
function startHelper(mode, args) {
  const child = spawn("powershell.exe", helperArgs(mode, args), {
    windowsHide: true,
    stdio: "ignore",
  });
  children.push(child);
  return child;
}
async function helper(mode, args) {
  await exec("powershell.exe", helperArgs(mode, args), { windowsHide: true, timeout: 20000 });
}
const app = spawn(resolve(binary), [], {
  windowsHide: true,
  stdio: "ignore",
  env: {
    ...process.env,
    LOCALAPPDATA: root,
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
  },
});
children.push(app);
let browser;
try {
  await until(async () => {
    assert.equal(
      app.exitCode,
      null,
      "Another resident PixelGrab may own the single-instance lock; exit it first.",
    );
    const response = await fetch(`http://127.0.0.1:${port}/json/list`);
    return (await response.json()).filter((p) => p.url.includes("tauri.localhost")).length === 3;
  }, "Production WebViews did not start");
  browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
  const context = browser.contexts()[0];
  const main = context.pages().find((p) => p.url() === "http://tauri.localhost/");
  const overlay = context.pages().find((p) => p.url().endsWith("/overlay.html"));
  const shelf = context.pages().find((p) => p.url().endsWith("/shelf.html"));
  const errors = [];
  context.on("page", (page) => page.on("pageerror", (error) => errors.push(error.message)));
  for (const page of context.pages()) page.on("pageerror", (error) => errors.push(error.message));
  const invoke = (page, command, args = {}) =>
    page.evaluate(({ command, args }) => window.__TAURI_INTERNALS__.invoke(command, args), {
      command,
      args,
    });
  const queue = async () => (await invoke(main, "get_shelf_queue_snapshot")).data;
  const cards = async () => {
    const q = await queue();
    return q.cards.concat(q.overflow);
  };
  const visible = (page, label) => invoke(page, "plugin:window|is_visible", { label });
  async function capture(full = false, hotkey = false) {
    const before = await overlay.locator("[data-testid=overlay]").getAttribute("data-capture-id");
    const count = (await cards()).length;
    const marker = join(root, `desktop-${count}-${Date.now()}`);
    const cover = startHelper("desktop", ["-OutputPath", marker, "-AppId", String(app.pid)]);
    try {
      await until(async () => {
        const ready = await access(`${marker}.ready`).then(
          () => true,
          () => false,
        );
        const unavailable = await access(`${marker}.unavailable`).then(
          () => true,
          () => false,
        );
        return ready || unavailable;
      }, "Desktop fixture did not start");
      const unavailable = await access(`${marker}.unavailable`).then(
        () => true,
        () => false,
      );
      assert.equal(
        unavailable,
        false,
        "Windows cannot read the controlled desktop; wake/connect the display before native acceptance",
      );
      if (hotkey) await helper("hotkey", []);
      else
        await main
          .getByRole("button", { name: full ? "Full screen" : "Capture area", exact: true })
          .evaluate((button) => button.click());
      if (full)
        await until(
          async () => (await cards()).length === count + 1,
          "Full-screen delivery failed",
        );
      else {
        await overlay.waitForFunction(
          (old) => {
            const current = document
              .querySelector("[data-testid=overlay]")
              ?.getAttribute("data-capture-id");
            return (
              current && current !== old && document.querySelector('[data-image-state="ready"]')
            );
          },
          before,
          { timeout: 15000 },
        );
      }
    } catch (error) {
      app.kill();
      await until(
        async () => app.exitCode !== null || app.signalCode !== null,
        "Test app did not stop after capture failure",
      );
      throw error;
    } finally {
      cover.kill();
    }
  }
  async function select(edit) {
    if (edit) await overlay.keyboard.down("Control");
    await overlay.mouse.move(100, 150);
    await overlay.mouse.down();
    await overlay.mouse.move(420, 370, { steps: 12 });
    await overlay.mouse.up();
    if (edit) await overlay.keyboard.up("Control");
  }
  async function compareImages(pathA, pathB) {
    const urls = await Promise.all(
      [pathA, pathB].map(
        async (path) => `data:image/png;base64,${(await readFile(path)).toString("base64")}`,
      ),
    );
    const result = await main.evaluate(async (sources) => {
      const decoded = await Promise.all(
        sources.map(async (src) => {
          const image = new Image();
          image.src = src;
          await image.decode();
          const canvas = document.createElement("canvas");
          canvas.width = image.width;
          canvas.height = image.height;
          const ctx = canvas.getContext("2d");
          ctx.drawImage(image, 0, 0);
          return {
            width: image.width,
            height: image.height,
            data: ctx.getImageData(0, 0, image.width, image.height).data,
          };
        }),
      );
      return (
        decoded[0].width === decoded[1].width &&
        decoded[0].height === decoded[1].height &&
        decoded[0].data.every((value, index) => value === decoded[1].data[index])
      );
    }, urls);
    assert.equal(result, true, "Exported image pixels differ");
  }

  async function checkFixture(path, annotated) {
    const src = `data:image/png;base64,${(await readFile(path)).toString("base64")}`;
    const pixels = await main.evaluate(async (source) => {
      const image = new Image();
      image.src = source;
      await image.decode();
      const canvas = document.createElement("canvas");
      canvas.width = image.width;
      canvas.height = image.height;
      const ctx = canvas.getContext("2d");
      ctx.drawImage(image, 0, 0);
      const data = ctx.getImageData(0, 0, image.width, image.height).data;
      let red = 0;
      for (let i = 0; i < data.length; i += 4) {
        if (data[i] === 229 && data[i + 1] === 59 && data[i + 2] === 59) red++;
      }
      return { width: image.width, height: image.height, first: [...data.slice(0, 4)], red };
    }, src);
    const scale = await overlay.evaluate(() => devicePixelRatio);
    assert.equal(pixels.width, Math.round(320 * scale));
    assert.equal(pixels.height, Math.round(220 * scale));
    assert.deepEqual(
      pixels.first,
      [255, 127, 80, 255],
      "Capture must contain the known coral desktop fixture",
    );
    assert.equal(pixels.red > 0, annotated, "Rectangle pixels must match the editable scene");
  }

  await capture();
  // Cross the actual display-watcher interval before selecting. This failed
  // in the original production adapter with "no frozen frame available".
  await delay(3500);
  await select(true);
  await overlay.getByTestId("tool-rectangle").click();
  await overlay.mouse.move(135, 185);
  await overlay.mouse.down();
  await overlay.mouse.move(350, 310, { steps: 8 });
  await overlay.mouse.up();
  await overlay.getByTestId("undo").click();
  await overlay.getByTestId("redo").click();
  assert.match(await overlay.getByTestId("selection").innerText(), /1 annotation/);
  await overlay.getByRole("button", { name: /^Save/ }).click();
  try {
    await helper("cancel", ["-AppId", String(app.pid)]);
  } catch (error) {
    console.info("Overlay Save As feedback:", await overlay.locator(".error").allTextContents());
    throw error;
  }
  await overlay.getByRole("button", { name: /^Save/ }).click();
  const overlaySaved = join(root, "overlay.png");
  await helper("save", ["-AppId", String(app.pid), "-OutputPath", overlaySaved]);
  await overlay.getByTestId("save-as-success").waitFor();
  await overlay.getByRole("button", { name: /Done/ }).click();
  await until(async () => (await cards()).length === 1, "Annotated capture was not delivered");
  assert.equal(await visible(overlay, "overlay"), false);
  assert.equal(await visible(main, "main"), false);
  assert.equal(await visible(shelf, "shelf"), true);
  await shelf.screenshot({ path: join(root, "floating-thumbnail.png") });
  const first = (await cards())[0];
  await checkFixture(first.pngPath, true);
  const original = await readFile(first.pngPath);
  await compareImages(overlaySaved, first.pngPath);
  const clipboard = join(root, "clipboard.png");
  await helper("clipboard", ["-OutputPath", clipboard]);
  await compareImages(clipboard, first.pngPath);
  console.info(
    "PASS delayed selection, annotation, undo/redo, Save As cancel/save, commit, clipboard pixels and floating thumbnail",
  );

  await shelf.getByTestId("shelf-edit").first().click();
  await main.locator('[data-image-state="ready"]').waitFor();
  await invoke(main, "plugin:window|set_size", {
    label: "main",
    value: { Logical: { width: 320, height: 680 } },
  });
  await main.waitForFunction(() => innerWidth <= 320);
  assert.equal(
    await main.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1),
    true,
    "Editor controls must fit a 320px window",
  );
  await invoke(main, "plugin:window|set_size", {
    label: "main",
    value: { Logical: { width: 840, height: 680 } },
  });
  await main.waitForFunction(() => innerWidth >= 830);
  await main.getByRole("button", { name: "100%", exact: true }).click();
  await main.getByRole("button", { name: "Fit", exact: true }).click();
  await main.keyboard.press("Control+a");
  await main.keyboard.press("Delete");
  await main.getByRole("button", { name: /Save as/ }).click();
  const revisionSaved = join(root, "revision.png");
  await helper("save", ["-AppId", String(app.pid), "-OutputPath", revisionSaved]);
  await until(async () => {
    await access(revisionSaved);
    return true;
  }, "Revision Save As failed");
  await checkFixture(revisionSaved, false);
  await compareImages(revisionSaved, join(dirname(first.pngPath), "source.png"));
  assert.deepEqual(
    await readFile(first.pngPath),
    original,
    "Editing must not mutate the original capture",
  );
  await main.getByRole("button", { name: /Copy and close/ }).click();
  await until(async () => (await cards()).length === 2, "Revision commit failed");
  assert.equal(await visible(main, "main"), false);
  console.info(
    "PASS thumbnail edit, editable-source round trip, zoom, editor Save As and copy/close",
  );

  await capture(false, true);
  await select(false);
  await until(async () => (await cards()).length === 3, "Subsequent area capture failed");
  await capture(true);
  assert.equal(await visible(overlay, "overlay"), false);
  console.info(
    "PASS subsequent area capture and full-screen capture deliver thumbnails without a main window",
  );

  await main
    .locator(`[data-shelf-id="${first.shelfId}"]`)
    .getByTestId("shelf-pin")
    .evaluate((button) => button.click());
  await until(
    async () => context.pages().some((p) => p.url().includes("pin.html")),
    "Pin window did not open",
  );
  const pin = context.pages().find((p) => p.url().includes("pin.html"));
  await pin.waitForSelector("img");
  await pin.getByRole("button", { name: "Close pin", exact: true }).click();
  await until(async () => pin.isClosed(), "Pin Close button did not close its native window");
  assert.deepEqual((await invoke(main, "list_pins")).data, []);
  console.info("PASS native pin window and close");

  const dropResult = join(root, "drop.json");
  const receiver = startHelper("receiver", ["-OutputPath", dropResult]);
  await until(async () => {
    await access(`${dropResult}.ready`);
    return true;
  }, "Drop receiver did not start");
  const position = await invoke(shelf, "plugin:window|outer_position", { label: "shelf" });
  const surface = await shelf.getByTestId("shelf-drag-surface").first().boundingBox();
  const dpr = await shelf.evaluate(() => devicePixelRatio);
  await helper("drag", [
    "-X",
    String(Math.round(position.x + (surface.x + surface.width / 2) * dpr)),
    "-Y",
    String(Math.round(position.y + (surface.y + 40) * dpr)),
    "-ToX",
    "300",
    "-ToY",
    "220",
  ]);
  await until(async () => {
    await access(dropResult);
    return true;
  }, "Native OLE drop did not deliver a file");
  const dropped = JSON.parse(await readFile(dropResult, "utf8"));
  const offered = Array.isArray(dropped) ? dropped[0] : dropped;
  await access(offered);
  assert.equal((await cards()).length, 4, "Sharing must retain reusable thumbnails");
  receiver.kill();
  assert.deepEqual(errors, []);
  console.info(
    "PASS real OLE drag to an external Windows drop target, readable PNG and retained thumbnails",
  );
} finally {
  try {
    await browser?.close();
  } finally {
    app.kill();
    await until(
      async () => app.exitCode !== null || app.signalCode !== null,
      "Test app did not stop during cleanup",
    );
    for (const child of children.reverse()) child.kill();
  }
}
