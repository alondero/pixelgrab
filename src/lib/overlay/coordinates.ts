import type { PhysicalPoint, PhysicalSize } from "$lib/ipc/types";

/** Convert a point in the frozen frame's physical pixels to stage CSS pixels. */
export function physicalToStagePoint(
  point: PhysicalPoint,
  sourceSize: PhysicalSize,
  stageSize: PhysicalSize,
): PhysicalPoint {
  return {
    x: point.x * (stageSize.width / sourceSize.width),
    y: point.y * (stageSize.height / sourceSize.height),
  };
}

/** Convert a stage CSS point back to physical pixels in the frozen frame. */
export function stageToPhysicalPoint(
  point: PhysicalPoint,
  sourceSize: PhysicalSize,
  stageSize: PhysicalSize,
): PhysicalPoint {
  return {
    x: point.x * (sourceSize.width / stageSize.width),
    y: point.y * (sourceSize.height / stageSize.height),
  };
}
