import { describe, expect, it } from "vitest";
import { physicalToStagePoint, stageToPhysicalPoint } from "./coordinates";

describe("overlay coordinate transforms", () => {
  it("maps physical annotation coordinates into a scaled stage", () => {
    expect(
      physicalToStagePoint(
        { x: 960, y: 540 },
        { width: 3840, height: 2160 },
        { width: 1920, height: 1080 },
      ),
    ).toEqual({ x: 480, y: 270 });
  });

  it("round-trips CSS pointer coordinates without losing the physical crop", () => {
    const physical = stageToPhysicalPoint(
      physicalToStagePoint(
        { x: 173, y: 811 },
        { width: 2560, height: 1440 },
        { width: 1536, height: 864 },
      ),
      { width: 2560, height: 1440 },
      { width: 1536, height: 864 },
    );
    expect(physical.x).toBeCloseTo(173);
    expect(physical.y).toBeCloseTo(811);
  });
});
