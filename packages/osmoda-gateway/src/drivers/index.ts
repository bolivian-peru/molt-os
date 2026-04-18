/**
 * Driver registry — gateway composes all available runtime drivers here.
 *
 * Adding a new driver is one import + one entry in `allDrivers`.
 */

import type { RuntimeDriver } from "./types.js";
import { claudeCodeDriver } from "./claude-code.js";
import { openClawDriver } from "./openclaw.js";

export const allDrivers: Record<string, RuntimeDriver> = {
  [claudeCodeDriver.name]: claudeCodeDriver,
  [openClawDriver.name]: openClawDriver,
};

export function getDriver(name: string): RuntimeDriver | null {
  return allDrivers[name] || null;
}

export function listDrivers(): RuntimeDriver[] {
  return Object.values(allDrivers);
}

export * from "./types.js";
