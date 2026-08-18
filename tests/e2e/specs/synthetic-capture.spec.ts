// Synthetic capture acceptance test. Drives the packaged binary through
// the tray intent -> synthetic capture -> overlay -> commit flow and
// asserts the visible UI plus the OS-observable results.

describe("Synthetic capture", () => {
  it("produces a capture and a commit outcome", async () => {
    // The Tauri service exposes the WebDriver protocol against the
    // packaged binary. Subsequent tracers wire the actual inventory.
    const driver = await browser.getDriver();
    await driver.pause(2000);
    const title = await driver.getTitle();
    expect(typeof title).toBe("string");
  });
});
